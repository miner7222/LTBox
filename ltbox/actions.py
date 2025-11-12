import os
import platform
import re
import shutil
import subprocess
import sys
import time
import xml.etree.ElementTree as ET
from datetime import datetime
from pathlib import Path
from typing import Optional, List, Dict, Any

from ltbox.constants import *
from ltbox import utils, device, imgpatch, downloader
from ltbox.downloader import ensure_magiskboot

def _scan_and_decrypt_xmls() -> List[Path]:
    OUTPUT_XML_DIR.mkdir(exist_ok=True)
    
    xmls = list(OUTPUT_XML_DIR.glob("rawprogram*.xml"))
    if not xmls:
        xmls = list(IMAGE_DIR.glob("rawprogram*.xml"))
    
    if not xmls:
        print("[*] No XML files found. Checking for .x files to decrypt...")
        x_files = list(IMAGE_DIR.glob("*.x"))
        
        if x_files:
            print(f"[*] Found {len(x_files)} .x files. Decrypting...")
            utils.check_dependencies() 
            for x_file in x_files:
                xml_name = x_file.stem + ".xml"
                out_path = OUTPUT_XML_DIR / xml_name
                if not out_path.exists():
                    print(f"  > Decrypting {x_file.name}...")
                    if imgpatch.decrypt_file(str(x_file), str(out_path)):
                        xmls.append(out_path)
                    else:
                        print(f"  [!] Failed to decrypt {x_file.name}")
        else:
            print("[!] No .xml or .x files found in 'image' folder.")
            print("[!] Dump requires partition information from these files.")
            print("    Please place firmware .xml or .x files into the 'image' folder.")
            return []
            
    return xmls

def _get_partition_params(target_label: str, xml_paths: List[Path]) -> Optional[Dict[str, Any]]:
    for xml_path in xml_paths:
        try:
            tree = ET.parse(xml_path)
            root = tree.getroot()
            for prog in root.findall('program'):
                label = prog.get('label', '').lower()
                if label == target_label.lower():
                    return {
                        'lun': prog.get('physical_partition_number'),
                        'start_sector': prog.get('start_sector'),
                        'num_sectors': prog.get('num_partition_sectors'),
                        'filename': prog.get('filename', ''),
                        'source_xml': xml_path.name
                    }
        except Exception as e:
            print(f"[!] Error parsing {xml_path.name}: {e}")
            
    return None

def _ensure_params_or_fail(label: str) -> Dict[str, Any]:
    xmls = _scan_and_decrypt_xmls()
    if not xmls:
        raise FileNotFoundError("No XML/.x files found for dump.")
        
    params = _get_partition_params(label, xmls)
    if not params:
        if label == "boot":
            params = _get_partition_params("boot_a", xmls)
            if not params:
                 params = _get_partition_params("boot_b", xmls)
                 
    if not params:
        print(f"[!] Error: Could not find partition info for '{label}' in XMLs.")
        raise ValueError(f"Partition '{label}' not found in XMLs")
        
    return params

def convert_images(device_model: Optional[str] = None, skip_adb: bool = False) -> None:
    utils.check_dependencies()
    
    print("--- Starting vendor_boot & vbmeta conversion process ---") 

    print("[*] Cleaning up old folders...")
    if OUTPUT_DIR.exists():
        shutil.rmtree(OUTPUT_DIR)
    print()

    print("--- Waiting for vendor_boot.img and vbmeta.img ---") 
    IMAGE_DIR.mkdir(exist_ok=True)
    required_files = ["vendor_boot.img", "vbmeta.img"]
    prompt = (
        "[STEP 1] Place the required firmware files for conversion\n"
        f"         (e.g., from your PRC firmware) into the '{IMAGE_DIR.name}' folder."
    )
    utils.wait_for_files(IMAGE_DIR, required_files, prompt)
    
    vendor_boot_src = IMAGE_DIR / "vendor_boot.img"
    vbmeta_src = IMAGE_DIR / "vbmeta.img"

    print("--- Backing up original images ---")
    vendor_boot_bak = BASE_DIR / "vendor_boot.bak.img"
    vbmeta_bak = BASE_DIR / "vbmeta.bak.img"
    
    try:
        shutil.copy(vendor_boot_src, vendor_boot_bak)
        shutil.copy(vbmeta_src, vbmeta_bak)
        print("[+] Backup complete.\n")
    except (IOError, OSError) as e:
        print(f"[!] Failed to copy input files: {e}", file=sys.stderr)
        raise

    print("--- Starting PRC/ROW Conversion ---")
    imgpatch.edit_vendor_boot(str(vendor_boot_bak))

    vendor_boot_prc = BASE_DIR / "vendor_boot_prc.img"
    print("\n[*] Verifying conversion result...")
    if not vendor_boot_prc.exists():
        print("[!] 'vendor_boot_prc.img' was not created. No changes made.")
        raise FileNotFoundError("vendor_boot_prc.img not created")
    print("[+] Conversion to PRC successful.\n")

    print("--- Extracting image information ---")
    vbmeta_info = imgpatch.extract_image_avb_info(vbmeta_bak)
    vendor_boot_info = imgpatch.extract_image_avb_info(vendor_boot_bak)
    print("[+] Information extracted.\n")

    if device_model and not skip_adb:
        print(f"[*] Validating firmware against device model '{device_model}'...")
        fingerprint_key = "com.android.build.vendor_boot.fingerprint"
        if fingerprint_key in vendor_boot_info:
            fingerprint = vendor_boot_info[fingerprint_key]
            print(f"  > Found firmware fingerprint: {fingerprint}")
            if device_model in fingerprint:
                print(f"[+] Success: Device model '{device_model}' found in firmware fingerprint.")
            else:
                print(f"[!] ERROR: Device model '{device_model}' NOT found in firmware fingerprint.")
                print("[!] The provided ROM does not match your device model. Aborting.")
                raise SystemExit("Firmware model mismatch")
        else:
            print(f"[!] Warning: Could not find fingerprint property '{fingerprint_key}' in vendor_boot.")
            print("[!] Skipping model validation.")
    
    print("--- Adding Hash Footer to vendor_boot ---")
    
    for key in ['partition_size', 'name', 'rollback', 'salt']:
        if key not in vendor_boot_info:
            if key == 'partition_size' and 'data_size' in vendor_boot_info:
                 vendor_boot_info['partition_size'] = vendor_boot_info['data_size']
            else:
                raise KeyError(f"Could not find '{key}' in '{vendor_boot_bak.name}' AVB info.")

    add_hash_footer_cmd = [
        str(PYTHON_EXE), str(AVBTOOL_PY), "add_hash_footer",
        "--image", str(vendor_boot_prc),
        "--partition_size", vendor_boot_info['partition_size'],
        "--partition_name", vendor_boot_info['name'],
        "--rollback_index", vendor_boot_info['rollback'],
        "--salt", vendor_boot_info['salt']
    ]
    
    if 'props_args' in vendor_boot_info:
        add_hash_footer_cmd.extend(vendor_boot_info['props_args'])
        print(f"[+] Restoring {len(vendor_boot_info['props_args']) // 2} properties for vendor_boot.")

    if 'flags' in vendor_boot_info:
        add_hash_footer_cmd.extend(["--flags", vendor_boot_info.get('flags', '0')])
        print(f"[+] Restoring flags for vendor_boot: {vendor_boot_info.get('flags', '0')}")

    utils.run_command(add_hash_footer_cmd)
    
    vbmeta_pubkey = vbmeta_info.get('pubkey_sha1')
    key_file = KEY_MAP.get(vbmeta_pubkey) 

    print(f"--- Remaking vbmeta.img ---")
    print("[*] Verifying vbmeta key...")
    if not key_file:
        print(f"[!] Public key SHA1 '{vbmeta_pubkey}' from vbmeta did not match known keys. Aborting.")
        raise KeyError(f"Unknown vbmeta public key: {vbmeta_pubkey}")
    print(f"[+] Matched {key_file.name}.\n")

    print("[*] Remaking 'vbmeta.img' using descriptors from backup...")
    vbmeta_img = BASE_DIR / "vbmeta.img"
    remake_cmd = [
        str(PYTHON_EXE), str(AVBTOOL_PY), "make_vbmeta_image",
        "--output", str(vbmeta_img),
        "--key", str(key_file),
        "--algorithm", vbmeta_info['algorithm'],
        "--padding_size", "8192",
        "--flags", vbmeta_info.get('flags', '0'),
        "--rollback_index", vbmeta_info.get('rollback', '0'),
        "--include_descriptors_from_image", str(vbmeta_bak),
        "--include_descriptors_from_image", str(vendor_boot_prc) 
    ]
        
    utils.run_command(remake_cmd)
    print()

    print("--- Finalizing ---")
    print("[*] Renaming final images...")
    final_vendor_boot = BASE_DIR / "vendor_boot.img"
    shutil.move(BASE_DIR / "vendor_boot_prc.img", final_vendor_boot)

    final_images = [final_vendor_boot, BASE_DIR / "vbmeta.img"]

    print(f"\n[*] Moving final images to '{OUTPUT_DIR.name}' folder...")
    OUTPUT_DIR.mkdir(exist_ok=True)
    for img in final_images:
        if img.exists(): 
            shutil.move(img, OUTPUT_DIR / img.name)

    print(f"\n[*] Moving backup files to '{BACKUP_DIR.name}' folder...")
    BACKUP_DIR.mkdir(exist_ok=True)
    for bak_file in BASE_DIR.glob("*.bak.img"):
        shutil.move(bak_file, BACKUP_DIR / bak_file.name)
    print()

    print("=" * 61)
    print("  SUCCESS!")
    print(f"  Final images have been saved to the '{OUTPUT_DIR.name}' folder.")
    print("=" * 61)

def root_boot_only() -> None:
    print(f"[*] Cleaning up old '{OUTPUT_ROOT_DIR.name}' folder...")
    if OUTPUT_ROOT_DIR.exists():
        shutil.rmtree(OUTPUT_ROOT_DIR)
    OUTPUT_ROOT_DIR.mkdir(exist_ok=True)
    print()
    
    utils.check_dependencies()
    magiskboot_exe = utils.get_platform_executable("magiskboot")
    ensure_magiskboot()

    if platform.system() != "Windows":
        os.chmod(magiskboot_exe, 0o755)

    print("--- Waiting for boot.img ---") 
    IMAGE_DIR.mkdir(exist_ok=True) 
    required_files = ["boot.img"]
    prompt = (
        "[STEP 1] Place your stock 'boot.img' file\n"
        f"         (e.g., from your firmware) into the '{IMAGE_DIR.name}' folder."
    )
    utils.wait_for_files(IMAGE_DIR, required_files, prompt)
    
    boot_img_src = IMAGE_DIR / "boot.img"
    boot_img = BASE_DIR / "boot.img" 
    
    try:
        shutil.copy(boot_img_src, boot_img)
        print(f"[+] Copied '{boot_img_src.name}' to main directory for processing.")
    except (IOError, OSError) as e:
        print(f"[!] Failed to copy '{boot_img_src.name}': {e}", file=sys.stderr)
        sys.exit(1)

    if not boot_img.exists():
        print("[!] 'boot.img' not found! Aborting.")
        sys.exit(1)

    shutil.copy(boot_img, BASE_DIR / "boot.bak.img")
    print("--- Backing up original boot.img ---")

    with utils.temporary_workspace(WORK_DIR):
        shutil.copy(boot_img, WORK_DIR / "boot.img")
        boot_img.unlink()
        
        patched_boot_path = imgpatch.patch_boot_with_root_algo(WORK_DIR, magiskboot_exe)

        if patched_boot_path and patched_boot_path.exists():
            print("\n--- Finalizing ---")
            final_boot_img = OUTPUT_ROOT_DIR / "boot.img"
            
            imgpatch.process_boot_image_avb(patched_boot_path)

            print(f"\n[*] Moving final image to '{OUTPUT_ROOT_DIR.name}' folder...")
            shutil.move(patched_boot_path, final_boot_img)

            print(f"\n[*] Moving backup file to '{BACKUP_DIR.name}' folder...")
            BACKUP_DIR.mkdir(exist_ok=True)
            for bak_file in BASE_DIR.glob("boot.bak.img"):
                shutil.move(bak_file, BACKUP_DIR / bak_file.name)
            print()

            print("=" * 61)
            print("  SUCCESS!")
            print(f"  Patched boot.img has been saved to the '{OUTPUT_ROOT_DIR.name}' folder.")
            print("=" * 61)
        else:
            print("[!] Patched boot image was not created. An error occurred during the process.", file=sys.stderr)

def select_country_code(prompt_message: str = "Please select a country from the list below:") -> str:
    print(f"\n--- {prompt_message.upper()} ---")

    if not COUNTRY_CODES:
        print("[!] Error: COUNTRY_CODES not found in constants.py. Aborting.", file=sys.stderr)
        raise ImportError("COUNTRY_CODES missing from constants.py")

    sorted_countries = sorted(COUNTRY_CODES.items(), key=lambda item: item[1])
    
    num_cols = 3
    col_width = 38 
    
    line_width = col_width * num_cols
    print("-" * line_width)
    
    for i in range(0, len(sorted_countries), num_cols):
        line = []
        for j in range(num_cols):
            idx = i + j
            if idx < len(sorted_countries):
                code, name = sorted_countries[idx]
                line.append(f"{idx+1:3d}. {name} ({code})".ljust(col_width))
        print("".join(line))
    print("-" * line_width)

    while True:
        try:
            choice = input(f"Enter the number (1-{len(sorted_countries)}): ")
            choice_idx = int(choice) - 1
            if 0 <= choice_idx < len(sorted_countries):
                selected_code = sorted_countries[choice_idx][0]
                selected_name = sorted_countries[choice_idx][1]
                print(f"[+] You selected: {selected_name} ({selected_code})")
                return selected_code
            else:
                print("[!] Invalid number. Please enter a number within the range.")
        except ValueError:
            print("[!] Invalid input. Please enter a number.")
        except (KeyboardInterrupt, EOFError):
            print("\n[!] Selection cancelled by user. Exiting.")
            sys.exit(1)

def edit_devinfo_persist() -> None:
    print("--- Starting devinfo & persist patching process ---")
    
    print("--- Waiting for devinfo.img / persist.img ---") 
    BACKUP_DIR.mkdir(exist_ok=True) 

    devinfo_img_src = BACKUP_DIR / "devinfo.img"
    persist_img_src = BACKUP_DIR / "persist.img"
    
    devinfo_img = BASE_DIR / "devinfo.img"
    persist_img = BASE_DIR / "persist.img"

    if not devinfo_img_src.exists() and not persist_img_src.exists():
        prompt = (
            "[STEP 1] Place 'devinfo.img' and/or 'persist.img'\n"
            f"         (e.g., from 'Dump' menu) into the '{BACKUP_DIR.name}' folder."
        )
        while not devinfo_img_src.exists() and not persist_img_src.exists():
            if platform.system() == "Windows":
                os.system('cls')
            else:
                os.system('clear')
            print("--- WAITING FOR FILES ---")
            print(prompt)
            print(f"\nPlease place at least one file in the '{BACKUP_DIR.name}' folder:")
            print(" - devinfo.img")
            print(" - persist.img")
            print("\nPress Enter when ready...")
            try:
                input()
            except EOFError:
                sys.exit(1)

    if devinfo_img_src.exists():
        shutil.copy(devinfo_img_src, devinfo_img)
    if persist_img_src.exists():
        shutil.copy(persist_img_src, persist_img)

    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    backup_critical_dir = BASE_DIR / f"backup_critical_{timestamp}"
    backup_critical_dir.mkdir(exist_ok=True)
    
    if devinfo_img.exists():
        shutil.copy(devinfo_img, backup_critical_dir)
    if persist_img.exists():
        shutil.copy(persist_img, backup_critical_dir)
    print(f"[+] Files copied and backed up to '{backup_critical_dir.name}'.\n")

    print(f"[*] Cleaning up old '{OUTPUT_DP_DIR.name}' folder...")
    if OUTPUT_DP_DIR.exists():
        shutil.rmtree(OUTPUT_DP_DIR)
    OUTPUT_DP_DIR.mkdir(exist_ok=True)

    print("[*] Detecting current region codes in images...")
    detected_codes = imgpatch.detect_region_codes()
    
    status_messages = []
    files_found = 0
    is_cn_detected = False
    
    display_order = ["persist.img", "devinfo.img"]
    
    for fname in display_order:
        if fname in detected_codes:
            code = detected_codes[fname]
            display_name = Path(fname).stem 
            
            if code:
                status_messages.append(f"{display_name}: {code}XX")
                files_found += 1
                if code == "CN":
                    is_cn_detected = True
            else:
                status_messages.append(f"{display_name}: null")
    
    print(f"\n[+] Detection Result:  {', '.join(status_messages)}")

    proceed = False
    
    if files_found == 0:
        print("[!] No region codes detected. Patching skipped.")
        devinfo_img.unlink(missing_ok=True)
        persist_img.unlink(missing_ok=True)
        return
    elif is_cn_detected:
        print("\n[*] CN Region detected. Automatically proceeding to region selection...")
        proceed = True
    else:
        print("\nDo you want to change the region code? (y/n)")
        choice = ""
        while choice not in ['y', 'n']:
            choice = input("    Enter choice: ").lower().strip()
        
        if choice == 'y':
            proceed = True
        else:
            print("[*] Operation cancelled. No changes made.")
            
            devinfo_img.unlink(missing_ok=True)
            persist_img.unlink(missing_ok=True)
            
            print("[*] Safety: Removing stock devinfo.img/persist.img from 'image' folder to prevent accidental flash.")
            (IMAGE_DIR / "devinfo.img").unlink(missing_ok=True)
            (IMAGE_DIR / "persist.img").unlink(missing_ok=True)
            
            return

    if proceed:
        target_map = detected_codes.copy()
        replacement_code = select_country_code("SELECT NEW REGION CODE")
        imgpatch.patch_region_codes(replacement_code, target_map)

        modified_devinfo = BASE_DIR / "devinfo_modified.img"
        modified_persist = BASE_DIR / "persist_modified.img"
        
        if modified_devinfo.exists():
            shutil.move(modified_devinfo, OUTPUT_DP_DIR / "devinfo.img")
        if modified_persist.exists():
            shutil.move(modified_persist, OUTPUT_DP_DIR / "persist.img")
            
        print(f"\n[*] Final images have been moved to '{OUTPUT_DP_DIR.name}' folder.")
    
    devinfo_img.unlink(missing_ok=True)
    persist_img.unlink(missing_ok=True)
    
    print("\n" + "=" * 61)
    print("  SUCCESS!")
    print(f"  Modified images are ready in the '{OUTPUT_DP_DIR.name}' folder.")
    print("=" * 61)

def modify_xml(wipe: int = 0, skip_dp: bool = False) -> None:
    print("--- Starting XML Modification Process ---")
    
    print("--- Waiting for 'image' folder ---")
    prompt = (
        "[STEP 1] Please copy the entire 'image' folder from your\n"
        "         unpacked Lenovo RSA firmware into the main directory."
    )
    utils.wait_for_directory(IMAGE_DIR, prompt)

    if OUTPUT_XML_DIR.exists():
        shutil.rmtree(OUTPUT_XML_DIR)
    OUTPUT_XML_DIR.mkdir(exist_ok=True)

    with utils.temporary_workspace(WORKING_DIR):
        print(f"\n[*] Created temporary '{WORKING_DIR.name}' folder.")
        try:
            imgpatch.modify_xml_algo(wipe=wipe)

            if not skip_dp:
                print("\n[*] Creating custom write XMLs for devinfo/persist...")

                src_persist_xml = OUTPUT_XML_DIR / "rawprogram_save_persist_unsparse0.xml"
                dest_persist_xml = OUTPUT_XML_DIR / "rawprogram_write_persist_unsparse0.xml"
                
                if src_persist_xml.exists():
                    try:
                        content = src_persist_xml.read_text(encoding='utf-8')
                        
                        content = re.sub(
                            r'(<program[^>]*\blabel="persist"[^>]*filename=")[^"]*(".*/>)',
                            r'\1persist.img\2',
                            content,
                            flags=re.IGNORECASE
                        )
                        content = re.sub(
                            r'(<program[^>]*filename=")[^"]*("[^>]*\blabel="persist"[^>]*/>)',
                            r'\1persist.img\2',
                            content,
                            flags=re.IGNORECASE
                        )
                        
                        dest_persist_xml.write_text(content, encoding='utf-8')
                        print(f"[+] Created '{dest_persist_xml.name}' in '{dest_persist_xml.parent.name}'.")
                    except Exception as e:
                        print(f"[!] Failed to create '{dest_persist_xml.name}': {e}", file=sys.stderr)
                else:
                    print(f"[!] Warning: '{src_persist_xml.name}' not found. Cannot create persist write XML.")

                src_devinfo_xml = OUTPUT_XML_DIR / "rawprogram4.xml"
                dest_devinfo_xml = OUTPUT_XML_DIR / "rawprogram4_write_devinfo.xml"
                
                if src_devinfo_xml.exists():
                    try:
                        content = src_devinfo_xml.read_text(encoding='utf-8')

                        content = re.sub(
                            r'(<program[^>]*\blabel="devinfo"[^>]*filename=")[^"]*(".*/>)',
                            r'\1devinfo.img\2',
                            content,
                            flags=re.IGNORECASE
                        )
                        content = re.sub(
                            r'(<program[^>]*filename=")[^"]*("[^>]*\blabel="devinfo"[^>]*/>)',
                            r'\1devinfo.img\2',
                            content,
                            flags=re.IGNORECASE
                        )
                        
                        dest_devinfo_xml.write_text(content, encoding='utf-8')
                        print(f"[+] Created '{dest_devinfo_xml.name}' in '{dest_devinfo_xml.parent.name}'.")
                    except Exception as e:
                        print(f"[!] Failed to create '{dest_devinfo_xml.name}': {e}", file=sys.stderr)
                else:
                    print(f"[!] Warning: '{src_devinfo_xml.name}' not found. Cannot create devinfo write XML.")

        except Exception as e:
            print(f"[!] Error during XML modification: {e}", file=sys.stderr)
            raise
        
        print(f"[*] Cleaned up temporary '{WORKING_DIR.name}' folder.")
    
    print("\n" + "=" * 61)
    print("  SUCCESS!")
    print(f"  Modified XML files are ready in the '{OUTPUT_XML_DIR.name}'.")
    print("  You can now run 'Flash EDL' (Menu 10).")
    print("=" * 61)

def disable_ota(skip_adb: bool = False) -> None:
    dev = device.DeviceController(skip_adb=skip_adb)
    if dev.skip_adb:
        print("[!] 'Disable OTA' was skipped as requested by Skip ADB setting.")
        return
    
    print("--- Starting Disable OTA Process ---")
    
    print("\n" + "="*61)
    print("  STEP 1/2: Waiting for ADB Connection")
    print("="*61)
    try:
        dev.wait_for_adb()
        print("[+] ADB device connected.")
    except Exception as e:
        print(f"[!] Error waiting for ADB device: {e}", file=sys.stderr)
        raise

    print("\n" + "="*61)
    print("  STEP 2/2: Disabling Lenovo OTA Service")
    print("="*61)
    
    command = [
        str(ADB_EXE), 
        "shell", "pm", "disable-user", "--user", "0", "com.lenovo.ota"
    ]
    
    print(f"[*] Running command: {' '.join(command)}")
    try:
        result = utils.run_command(command, capture=True)
        if "disabled" in result.stdout.lower() or "already disabled" in result.stdout.lower():
            print("[+] Success: OTA service (com.lenovo.ota) is now disabled.")
            print(result.stdout.strip())
        else:
            print("[!] Command executed, but result was unexpected.")
            print(f"Stdout: {result.stdout.strip()}")
            if result.stderr:
                print(f"Stderr: {result.stderr.strip()}", file=sys.stderr)
    except Exception as e:
        print(f"[!] An error occurred while running the command: {e}", file=sys.stderr)
        raise

    print("\n--- Disable OTA Process Finished ---")

def read_edl(skip_adb: bool = False, skip_reset: bool = False, additional_targets: Optional[List[str]] = None) -> None:
    print("--- Starting Dump Process (fh_loader) ---")
    
    dev = device.DeviceController(skip_adb=skip_adb)
    port = dev.setup_edl_connection()
    
    try:
        dev.load_firehose_programmer(EDL_LOADER_FILE, port)
        time.sleep(2)
    except Exception as e:
        print(f"[!] Warning: Programmer loading issue (might be already loaded): {e}")

    BACKUP_DIR.mkdir(exist_ok=True)
    
    targets = ["devinfo", "persist"]

    if additional_targets:
        targets.extend(additional_targets)
        print(f"[*] Extended dump targets: {', '.join(targets)}")
    
    for target in targets:
        out_file = BACKUP_DIR / f"{target}.img"
        print(f"\n[*] Preparing to dump '{target}'...")
        
        try:
            params = _ensure_params_or_fail(target)
            print(f"  > Found info in {params['source_xml']}: LUN={params['lun']}, Start={params['start_sector']}")
            
            dev.fh_loader_read_part(
                port=port,
                output_filename=str(out_file),
                lun=params['lun'],
                start_sector=params['start_sector'],
                num_sectors=params['num_sectors']
            )
            print(f"[+] Successfully read '{target}' to '{out_file.name}'.")
            
        except (ValueError, FileNotFoundError) as e:
            print(f"[!] Skipping '{target}': {e}")
        except Exception as e:
            print(f"[!] Failed to read '{target}': {e}", file=sys.stderr)

        print("[*] Waiting 5 seconds for stability...")
        time.sleep(5)

    if not skip_reset:
        print("\n[*] Resetting device to system...")
        dev.fh_loader_reset(port)
        print("[+] Reset command sent.")
        print("[*] Waiting 10 seconds for stability...")
        time.sleep(10)
    else:
        print("\n[*] Skipping reset as requested (Device remains in EDL).")

    print(f"\n--- Dump Process Finished ---")
    print(f"[*] Files saved to: {BACKUP_DIR.name}")

def read_edl_fhloader(skip_adb: bool = False, skip_reset: bool = False, additional_targets: Optional[List[str]] = None) -> None:
    return read_edl(skip_adb, skip_reset=skip_reset, additional_targets=additional_targets)

def write_edl(skip_reset: bool = False, skip_reset_edl: bool = False) -> None:
    print("--- Starting Write Process (Fastboot) ---")

    skip_adb_val = os.environ.get('SKIP_ADB') == '1'
    dev = device.DeviceController(skip_adb=skip_adb_val)

    if not OUTPUT_DP_DIR.exists():
        print(f"[!] Error: Patched images folder '{OUTPUT_DP_DIR.name}' not found.", file=sys.stderr)
        print("[!] Please run 'Patch devinfo/persist' (Menu 3) first to generate the modified images.", file=sys.stderr)
        raise FileNotFoundError(f"{OUTPUT_DP_DIR.name} not found.")
    print(f"[+] Found patched images folder: '{OUTPUT_DP_DIR.name}'.")

    if not dev.skip_adb:
        print("[*] checking device state...")
        
        if dev.check_fastboot_device(silent=True):
            print("[+] Device is already in Fastboot mode.")
        
        else:
            edl_port = dev.check_edl_device(silent=True)
            if edl_port:
                print(f"[!] Device found in EDL mode ({edl_port}).")
                print("[*] Resetting to System via fh_loader to prepare for Fastboot...")
                try:
                    dev.fh_loader_reset(edl_port)
                    print("[+] Reset command sent. Waiting for device to boot...")
                    time.sleep(10)
                except Exception as e:
                    print(f"[!] Warning: Failed to reset from EDL: {e}")
            
            try:
                dev.wait_for_adb()
                dev.reboot_to_bootloader()
                time.sleep(10)
            except Exception as e:
                print(f"[!] Error requesting reboot to bootloader: {e}")
                print("[!] Please manually enter Fastboot mode if the script hangs.")

    else:
        print("\n" + "="*61)
        print("  [SKIP ADB ACTIVE]")
        print("  Please manually boot your device into FASTBOOT mode.")
        print("  (Power + Volume Down usually works)")
        print("="*61 + "\n")
        input("  Press Enter when device is in Fastboot mode...")

    dev.wait_for_fastboot()

    patched_devinfo = OUTPUT_DP_DIR / "devinfo.img"
    patched_persist = OUTPUT_DP_DIR / "persist.img"

    if not patched_devinfo.exists() and not patched_persist.exists():
         print(f"[!] Error: Neither 'devinfo.img' nor 'persist.img' found inside '{OUTPUT_DP_DIR.name}'.", file=sys.stderr)
         raise FileNotFoundError(f"No patched images found in {OUTPUT_DP_DIR.name}.")

    try:
        if patched_devinfo.exists():
            print(f"\n[*] Flashing 'devinfo' partition via Fastboot...")
            utils.run_command([str(FASTBOOT_EXE), "flash", "devinfo", str(patched_devinfo)])
            print("[+] Successfully flashed 'devinfo'.")
        else:
            print(f"\n[*] 'devinfo.img' not found. Skipping.")

        if patched_persist.exists():
            print(f"\n[*] Flashing 'persist' partition via Fastboot...")
            utils.run_command([str(FASTBOOT_EXE), "flash", "persist", str(patched_persist)])
            print("[+] Successfully flashed 'persist'.")
        else:
            print(f"\n[*] 'persist.img' not found. Skipping.")

    except subprocess.CalledProcessError as e:
        print(f"[!] Fastboot flashing failed: {e}", file=sys.stderr)
        raise

    if not skip_reset:
        print("\n[*] Rebooting device...")
        try:
            utils.run_command([str(FASTBOOT_EXE), "reboot"])
        except Exception as e:
            print(f"[!] Warning: Failed to reboot: {e}")
    else:
        print("\n[*] Skipping reboot as requested.")

    print("\n--- Write Process Finished ---")

def read_anti_rollback(dumped_boot_path: Path, dumped_vbmeta_path: Path) -> Tuple[str, int, int]:
    print("--- Anti-Rollback Status Check ---")
    utils.check_dependencies()
    
    current_boot_rb = 0
    current_vbmeta_rb = 0
    
    print("\n--- [STEP 1] Parsing Rollback Indices from DUMPED IMAGES ---")
    try:
        if not dumped_boot_path.exists() or not dumped_vbmeta_path.exists():
            raise FileNotFoundError("Dumped boot/vbmeta images not found.")
        
        print(f"[*] Reading from: {dumped_boot_path.name}")
        boot_info = imgpatch.extract_image_avb_info(dumped_boot_path)
        current_boot_rb = int(boot_info.get('rollback', '0'))
        
        print(f"[*] Reading from: {dumped_vbmeta_path.name}")
        vbmeta_info = imgpatch.extract_image_avb_info(dumped_vbmeta_path)
        current_vbmeta_rb = int(vbmeta_info.get('rollback', '0'))
        
    except Exception as e:
        print(f"[!] Error extracting AVB info from dumps: {e}", file=sys.stderr)
        print(f"\n--- Status Check Complete: ERROR ---")
        return 'ERROR', 0, 0

    print(f"  > Current Device Boot Index: {current_boot_rb}")
    print(f"  > Current Device VBMeta System Index: {current_vbmeta_rb}")

    print("\n--- [STEP 2] Comparing New ROM Indices ---")
    print("\n[*] Extracting new ROM's rollback indices (from 'image' folder)...")
    new_boot_img = IMAGE_DIR / "boot.img"
    new_vbmeta_img = IMAGE_DIR / "vbmeta_system.img"

    if not new_boot_img.exists() or not new_vbmeta_img.exists():
        print(f"[!] Error: 'boot.img' or 'vbmeta_system.img' not found in '{IMAGE_DIR.name}' folder.")
        print(f"\n--- Status Check Complete: MISSING_NEW ---")
        return 'MISSING_NEW', 0, 0
        
    new_boot_rb = 0
    new_vbmeta_rb = 0
    try:
        new_boot_info = imgpatch.extract_image_avb_info(new_boot_img)
        new_boot_rb = int(new_boot_info.get('rollback', '0'))
        
        new_vbmeta_info = imgpatch.extract_image_avb_info(new_vbmeta_img)
        new_vbmeta_rb = int(new_vbmeta_info.get('rollback', '0'))
    except Exception as e:
        print(f"[!] Error reading new image info: {e}. Please check files.", file=sys.stderr)
        print(f"\n--- Status Check Complete: ERROR ---")
        return 'ERROR', 0, 0

    print(f"  > New ROM's Boot Index: {new_boot_rb}")
    print(f"  > New ROM's VBMeta System Index: {new_vbmeta_rb}")

    if new_boot_rb == current_boot_rb and new_vbmeta_rb == current_vbmeta_rb:
        print("\n[+] Indices are identical. No Anti-Rollback patch needed.")
        status = 'MATCH'
    else:
        print("\n[*] Indices are different (higher or lower). Patching is REQUIRED.")
        status = 'NEEDS_PATCH'
    
    print(f"\n--- Status Check Complete: {status} ---")
    return status, current_boot_rb, current_vbmeta_rb

def patch_anti_rollback(comparison_result: Tuple[str, int, int]) -> None:
    print("--- Anti-Rollback Patcher ---")
    utils.check_dependencies()

    if OUTPUT_ANTI_ROLLBACK_DIR.exists():
        shutil.rmtree(OUTPUT_ANTI_ROLLBACK_DIR)
    OUTPUT_ANTI_ROLLBACK_DIR.mkdir(exist_ok=True)
    
    try:
        if comparison_result:
            print("[*] Using pre-computed Anti-Rollback status...")
            status, current_boot_rb, current_vbmeta_rb = comparison_result
        else:
            print("[!] No comparison result provided. Aborting.")
            return

        if status != 'NEEDS_PATCH':
            print("\n[!] No patching is required or files are missing. Aborting patch.")
            return

        print("\n--- [STEP 3] Patching New ROM ---")
        
        imgpatch.patch_chained_image_rollback(
            image_name="boot.img",
            current_rb_index=current_boot_rb,
            new_image_path=(IMAGE_DIR / "boot.img"),
            patched_image_path=(OUTPUT_ANTI_ROLLBACK_DIR / "boot.img")
        )
        
        print("-" * 20)
        
        imgpatch.patch_vbmeta_image_rollback(
            image_name="vbmeta_system.img",
            current_rb_index=current_vbmeta_rb,
            new_image_path=(IMAGE_DIR / "vbmeta_system.img"),
            patched_image_path=(OUTPUT_ANTI_ROLLBACK_DIR / "vbmeta_system.img")
        )

        print("\n" + "=" * 61)
        print("  SUCCESS!")
        print(f"  Anti-rollback patched images are in '{OUTPUT_ANTI_ROLLBACK_DIR.name}'.")
        print("  You can now run 'Write Anti-Rollback' (Menu 8).")
        print("=" * 61)

    except Exception as e:
        print(f"\n[!] An error occurred during patching: {e}", file=sys.stderr)
        shutil.rmtree(OUTPUT_ANTI_ROLLBACK_DIR) 

def write_anti_rollback(skip_reset: bool = False) -> None:
    print("--- Starting Anti-Rollback Write Process ---")

    boot_img = OUTPUT_ANTI_ROLLBACK_DIR / "boot.img"
    vbmeta_img = OUTPUT_ANTI_ROLLBACK_DIR / "vbmeta_system.img"

    if not boot_img.exists() or not vbmeta_img.exists():
        print(f"[!] Error: Patched images not found in '{OUTPUT_ANTI_ROLLBACK_DIR.name}'.", file=sys.stderr)
        print("[!] Please run 'Patch Anti-Rollback' (Menu 7) first.", file=sys.stderr)
        raise FileNotFoundError(f"Patched images not found in {OUTPUT_ANTI_ROLLBACK_DIR.name}")
    print(f"[+] Found patched images folder: '{OUTPUT_ANTI_ROLLBACK_DIR.name}'.")

    dev = device.DeviceController(skip_adb=False)
    
    if not skip_reset:
        dev.setup_edl_connection()
    
    try:
        print(f"\n[*] Attempting to write 'boot' partition...")
        dev.edl_write_part(EDL_LOADER_FILE, "boot_a", boot_img)
        print("[+] Successfully wrote 'boot'.")

        print(f"\n[*] Attempting to write 'vbmeta_system' partition...")
        dev.edl_write_part(EDL_LOADER_FILE, "vbmeta_system_a", vbmeta_img)
        print("[+] Successfully wrote 'vbmeta_system'.")

        if not skip_reset:
            print("\n[*] Operations complete. Resetting device...")
            dev.edl_reset(EDL_LOADER_FILE)
            print("[+] Device reset command sent.")
        else:
            print("\n[*] Operations complete. Skipping device reset as requested.")

    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] An error occurred during the EDL write operation: {e}", file=sys.stderr)
        raise
    
    print("\n--- Anti-Rollback Write Process Finished ---")

def flash_edl(skip_reset: bool = False, skip_reset_edl: bool = False, skip_dp: bool = False) -> None:
    print("--- Starting Full EDL Flash Process ---")

    skip_adb_val = os.environ.get('SKIP_ADB') == '1'
    dev = device.DeviceController(skip_adb=skip_adb_val)
    
    if not IMAGE_DIR.is_dir() or not any(IMAGE_DIR.iterdir()):
        print(f"[!] Error: The '{IMAGE_DIR.name}' folder is missing or empty.")
        print("[!] Please run 'Modify XML for Update' (Menu 9) first.")
        raise FileNotFoundError(f"{IMAGE_DIR.name} is missing or empty.")
        
    loader_path = EDL_LOADER_FILE
    if not loader_path.exists():
        print(f"[!] Error: EDL Loader '{loader_path.name}' not found in '{IMAGE_DIR.name}' folder.")
        print("[!] Please copy it to the 'image' folder (from firmware).")
        raise FileNotFoundError(f"{loader_path.name} not found in {IMAGE_DIR.name}")

    if not skip_reset_edl:
        print("\n" + "="*61)
        print("  WARNING: PROCEEDING WILL OVERWRITE FILES IN YOUR 'image'")
        print("           FOLDER WITH ANY PATCHED FILES YOU HAVE CREATED")
        print("           (e.g., from Menu 1, 5, 7, or 9).")
        print("="*61 + "\n")
        
        choice = ""
        while choice not in ['y', 'n']:
            choice = input("Are you sure you want to continue? (y/n): ").lower().strip()

        if choice == 'n':
            print("[*] Operation cancelled.")
            return

    print("\n[*] Copying patched files to 'image' folder (overwriting)...")
    output_folders_to_copy = [
        OUTPUT_DIR, 
        OUTPUT_ANTI_ROLLBACK_DIR,
        OUTPUT_XML_DIR
    ]
    
    copied_count = 0
    for folder in output_folders_to_copy:
        if folder.exists():
            try:
                shutil.copytree(folder, IMAGE_DIR, dirs_exist_ok=True)
                print(f"  > Copied contents of '{folder.name}' to '{IMAGE_DIR.name}'.")
                copied_count += 1
            except Exception as e:
                print(f"[!] Error copying files from {folder.name}: {e}", file=sys.stderr)
    
    if not skip_dp:
        if OUTPUT_DP_DIR.exists():
            try:
                shutil.copytree(OUTPUT_DP_DIR, IMAGE_DIR, dirs_exist_ok=True)
                print(f"  > Copied contents of '{OUTPUT_DP_DIR.name}' to '{IMAGE_DIR.name}'.")
                copied_count += 1
            except Exception as e:
                print(f"[!] Error copying files from {OUTPUT_DP_DIR.name}: {e}", file=sys.stderr)
        else:
            print(f"[*] '{OUTPUT_DP_DIR.name}' not found. Skipping devinfo/persist copy.")
    else:
        print(f"[*] Skipping devinfo/persist copy as requested.")

    if copied_count == 0:
        print("[*] No 'output*' folders found. Proceeding with files already in 'image' folder.")

    port = dev.setup_edl_connection()

    raw_xmls = [f for f in IMAGE_DIR.glob("rawprogram*.xml") if f.name != "rawprogram0.xml"]
    patch_xmls = list(IMAGE_DIR.glob("patch*.xml"))
    
    persist_write_xml = IMAGE_DIR / "rawprogram_write_persist_unsparse0.xml"
    persist_save_xml = IMAGE_DIR / "rawprogram_save_persist_unsparse0.xml"
    devinfo_write_xml = IMAGE_DIR / "rawprogram4_write_devinfo.xml"
    devinfo_original_xml = IMAGE_DIR / "rawprogram4.xml"

    has_patched_persist = (OUTPUT_DP_DIR / "persist.img").exists()
    has_patched_devinfo = (OUTPUT_DP_DIR / "devinfo.img").exists()

    if persist_write_xml.exists() and has_patched_persist and not skip_dp:
        print("[+] Using 'rawprogram_write_persist_unsparse0.xml' for persist flash (Patched).")
        raw_xmls = [xml for xml in raw_xmls if xml.name != persist_save_xml.name]
    else:
        if persist_write_xml.exists() and any(xml.name == persist_write_xml.name for xml in raw_xmls):
             print("[*] Skipping 'persist' flash (Not patched, preserving device data).")
             raw_xmls = [xml for xml in raw_xmls if xml.name != persist_write_xml.name]

    if devinfo_write_xml.exists() and has_patched_devinfo and not skip_dp:
        print("[+] Using 'rawprogram4_write_devinfo.xml' for devinfo flash (Patched).")
        raw_xmls = [xml for xml in raw_xmls if xml.name != devinfo_original_xml.name]
    else:
        if devinfo_write_xml.exists() and any(xml.name == devinfo_write_xml.name for xml in raw_xmls):
             print("[*] Skipping 'devinfo' flash (Not patched, preserving device data).")
             raw_xmls = [xml for xml in raw_xmls if xml.name != devinfo_write_xml.name]

    if not raw_xmls or not patch_xmls:
        print(f"[!] Error: 'rawprogram*.xml' (excluding rawprogram0.xml) or 'patch*.xml' files not found in '{IMAGE_DIR.name}'.")
        print(f"[!] Cannot flash. Please run XML modification first.")
        raise FileNotFoundError(f"Missing essential XML flash files in {IMAGE_DIR.name}")
        
    print("\n--- [STEP 1] Flashing all images via rawprogram (fh_loader) ---")
    
    try:
        dev.edl_rawprogram(loader_path, "UFS", raw_xmls, patch_xmls, port)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] An error occurred during main flash: {e}", file=sys.stderr)
        print("[!] The device may be in an unstable state. Do not reboot manually.")
        raise
        
    print("\n--- [STEP 2] Cleaning up temporary images ---")
    if not skip_dp:
        try:
            (IMAGE_DIR / "devinfo.img").unlink(missing_ok=True)
            (IMAGE_DIR / "persist.img").unlink(missing_ok=True)
            print("[+] Removed devinfo.img and persist.img from 'image' folder.")
        except OSError as e:
            print(f"[!] Error cleaning up images: {e}", file=sys.stderr)

    if not skip_reset:
        print("\n--- [STEP 3] Final step: Resetting device to system ---")
        try:
            print("[*] Waiting 5 seconds for stability...")
            time.sleep(5)
            
            print("[*] Attempting to reset device via fh_loader...")
            dev.fh_loader_reset(port)
            print("[+] Device reset command sent.")
        except (subprocess.CalledProcessError, FileNotFoundError, Exception) as e:
             print(f"[!] Failed to reset device: {e}", file=sys.stderr)
    else:
        print("[*] Skipping final device reset as requested.")

    if not skip_reset:
        print("\n--- Full EDL Flash Process Finished ---")

def _fh_loader_write_part(port, image_path, lun, start_sector):
    if not FH_LOADER_EXE.exists():
        raise FileNotFoundError(f"fh_loader.exe not found at {FH_LOADER_EXE}")
        
    port_str = f"\\\\.\\{port}"
    cmd = [
        str(FH_LOADER_EXE),
        f"--port={port_str}",
        f"--sendimage={image_path}",
        f"--lun={lun}",
        f"--start_sector={start_sector}",
        "--zlpawarehost=1",
        "--noprompt",
        "--memoryname=UFS"
    ]
    print(f"[*] Flashing {image_path.name} to LUN:{lun} @ {start_sector}...")
    utils.run_command(cmd)

def root_device(skip_adb=False):
    print("--- Starting Root Device Process (EDL Mode) ---")
    
    if OUTPUT_ROOT_DIR.exists():
        shutil.rmtree(OUTPUT_ROOT_DIR)
    OUTPUT_ROOT_DIR.mkdir(exist_ok=True)
    BACKUP_BOOT_DIR.mkdir(exist_ok=True)

    utils.check_dependencies()
    
    magiskboot_exe = utils.get_platform_executable("magiskboot")
    ensure_magiskboot()

    dev = device.DeviceController(skip_adb=skip_adb)

    print("\n--- [STEP 1/6] Waiting for ADB Connection ---")
    dev.wait_for_adb()

    if not skip_adb:
        print("\n[*] Checking & Installing KernelSU Next (Spoofed) APK...")
        downloader.download_ksu_apk(BASE_DIR)
        
        ksu_apks = list(BASE_DIR.glob("*spoofed*.apk"))
        if ksu_apks:
            apk_path = ksu_apks[0]
            print(f"[*] Installing {apk_path.name} via ADB...")
            try:
                utils.run_command([str(ADB_EXE), "install", "-r", str(apk_path)])
                print("[+] APK installed successfully.")
            except Exception as e:
                print(f"[!] Failed to install APK: {e}")
                print("[!] Proceeding with root process anyway...")
        else:
            print("[!] Spoofed APK not found. Skipping installation.")
    
    print("\n--- [STEP 2/6] Rebooting to EDL Mode ---")
    port = dev.setup_edl_connection()
    try:
        dev.load_firehose_programmer(EDL_LOADER_FILE, port)
        time.sleep(2)
    except Exception as e:
        print(f"[!] Warning: Programmer loading issue: {e}")

    print("\n--- [STEP 3/6] Dumping boot_a partition ---")
    
    params = None
    final_boot_img = OUTPUT_ROOT_DIR / "boot.img"
    
    with utils.temporary_workspace(WORKING_BOOT_DIR):
        dumped_boot_img = WORKING_BOOT_DIR / "boot.img"
        backup_boot_img = BACKUP_BOOT_DIR / "boot.img"
        base_boot_bak = BASE_DIR / "boot.bak.img"

        try:
            params = _ensure_params_or_fail("boot")
            print(f"  > Found info in {params['source_xml']}: LUN={params['lun']}, Start={params['start_sector']}")
            dev.fh_loader_read_part(
                port=port,
                output_filename=str(dumped_boot_img),
                lun=params['lun'],
                start_sector=params['start_sector'],
                num_sectors=params['num_sectors']
            )
            print(f"[+] Successfully read 'boot' to '{dumped_boot_img}'.")
        except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
            print(f"[!] Failed to read 'boot': {e}", file=sys.stderr)
            raise

        print(f"[*] Backing up original boot.img to '{backup_boot_img.parent.name}' folder...")
        shutil.copy(dumped_boot_img, backup_boot_img)
        print(f"[*] Creating temporary backup for AVB processing...")
        shutil.copy(dumped_boot_img, base_boot_bak)
        print("[+] Backups complete.")

        print("\n[*] Dumping complete. Resetting to System to clear EDL state...")
        dev.fh_loader_reset(port)
        
        print("\n--- [STEP 4/6] Patching dumped boot.img ---")
        patched_boot_path = imgpatch.patch_boot_with_root_algo(WORKING_BOOT_DIR, magiskboot_exe)

        if not (patched_boot_path and patched_boot_path.exists()):
            print("[!] Patched boot image was not created. An error occurred.", file=sys.stderr)
            base_boot_bak.unlink(missing_ok=True)
            sys.exit(1)

        print("\n--- [STEP 5/6] Processing AVB Footer ---")
        try:
            imgpatch.process_boot_image_avb(patched_boot_path)
        except Exception as e:
            print(f"[!] Failed to process AVB footer: {e}", file=sys.stderr)
            base_boot_bak.unlink(missing_ok=True)
            raise

        shutil.move(patched_boot_path, final_boot_img)
        print(f"[+] Patched boot image saved to '{final_boot_img.parent.name}' folder.")

        base_boot_bak.unlink(missing_ok=True)

    print("\n--- [STEP 6/6] Flashing patched boot.img via EDL ---")
    
    if not skip_adb:
        print("[*] Waiting for device to boot to System (ADB) to ensure clean state...")
        dev.wait_for_adb()
        print("[*] Rebooting to EDL for flashing...")
        port = dev.setup_edl_connection()
    else:
        print("[!] Skip ADB is ON.")
        print("[!] Please manually reboot your device to EDL mode now.")
        port = dev.wait_for_edl()

    try:
        dev.load_firehose_programmer(EDL_LOADER_FILE, port)
        time.sleep(2)
    except Exception as e:
        print(f"[!] Warning: Programmer loading issue: {e}")

    if not params:
         params = _ensure_params_or_fail("boot")

    try:
        _fh_loader_write_part(
            port=port,
            image_path=final_boot_img,
            lun=params['lun'],
            start_sector=params['start_sector']
        )
        print("[+] Successfully flashed 'boot.img' via EDL.")
        
        print("\n[*] Resetting to system...")
        dev.fh_loader_reset(port)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] An error occurred during EDL flash: {e}", file=sys.stderr)
        raise

    print("\n--- Root Device Process Finished ---")

def unroot_device(skip_adb=False):
    print("--- Starting Unroot Device Process (EDL Mode) ---")
    
    backup_boot_file = BACKUP_BOOT_DIR / "boot.img"
    BACKUP_BOOT_DIR.mkdir(exist_ok=True)
    
    print("\n--- [STEP 1/4] Checking Requirements ---")
    if not list(IMAGE_DIR.glob("rawprogram*.xml")) and not list(IMAGE_DIR.glob("*.x")):
         print(f"[!] Error: No firmware XMLs found in '{IMAGE_DIR.name}'.")
         print("[!] Unroot via EDL requires partition info from firmware XMLs.")
         prompt = (
            "[STEP 1] Please copy the entire 'image' folder from your\n"
            "         unpacked Lenovo RSA firmware into the main directory."
         )
         utils.wait_for_directory(IMAGE_DIR, prompt)

    print("\n--- [STEP 2/4] Checking for backup boot.img ---")
    if not backup_boot_file.exists():
        prompt = (
            "[!] Backup file 'boot.img' not found.\n"
            f"    Please place your stock 'boot.img' (from your current firmware)\n"
            f"    into the '{BACKUP_BOOT_DIR.name}' folder."
        )
        utils.wait_for_files(BACKUP_BOOT_DIR, ["boot.img"], prompt)
    
    print("[+] Stock backup 'boot.img' found.")

    dev = device.DeviceController(skip_adb=skip_adb)

    if not skip_adb:
        print("\n[STEP 3/4] Rebooting to EDL Mode...")
        dev.wait_for_adb()
        port = dev.setup_edl_connection()
    else:
        print("\n[STEP 3/4] Waiting for EDL Connection (Manual)...")
        port = dev.wait_for_edl()

    try:
        dev.load_firehose_programmer(EDL_LOADER_FILE, port)
        time.sleep(2)
    except Exception as e:
        print(f"[!] Warning: Programmer loading issue: {e}")

    print("\n--- [STEP 4/4] Flashing stock boot.img via EDL ---")
    try:
        params = _ensure_params_or_fail("boot")
        print(f"  > Found info in {params['source_xml']}: LUN={params['lun']}, Start={params['start_sector']}")
        
        _fh_loader_write_part(
            port=port,
            image_path=backup_boot_file,
            lun=params['lun'],
            start_sector=params['start_sector']
        )
        print("[+] Successfully flashed stock 'boot.img'.")
        
        print("\n[*] Resetting to system...")
        dev.fh_loader_reset(port)
        
    except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
        print(f"[!] An error occurred during EDL flash: {e}", file=sys.stderr)
        raise

    print("\n--- Unroot Device Process Finished ---")