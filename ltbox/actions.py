import os
import platform
import re
import shutil
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path

from ltbox.constants import *
from ltbox import utils, device, imgpatch, downloader

# --- Patch Actions ---

def convert_images(device_model=None):
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

    if device_model:
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
    
def root_boot_only():
    print(f"[*] Cleaning up old '{OUTPUT_ROOT_DIR.name}' folder...")
    if OUTPUT_ROOT_DIR.exists():
        shutil.rmtree(OUTPUT_ROOT_DIR)
    OUTPUT_ROOT_DIR.mkdir(exist_ok=True)
    print()
    
    utils.check_dependencies()

    magiskboot_exe = utils.get_platform_executable("magiskboot")
    fetch_exe = utils.get_platform_executable("fetch")
    
    if not fetch_exe.exists():
         print(f"[!] '{fetch_exe.name}' not found. Please run install.bat")
         sys.exit(1)

    downloader._ensure_magiskboot(fetch_exe, magiskboot_exe)

    if platform.system() != "Windows":
        os.chmod(magiskboot_exe, 0o755)
        os.chmod(fetch_exe, 0o755)

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

    if WORK_DIR.exists():
        shutil.rmtree(WORK_DIR)
    WORK_DIR.mkdir()
    shutil.copy(boot_img, WORK_DIR / "boot.img")
    boot_img.unlink()
    
    patched_boot_path = imgpatch.patch_boot_with_root_algo(WORK_DIR, magiskboot_exe, fetch_exe)

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

def select_country_code(prompt_message="Please select a country from the list below:"):
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

def edit_devinfo_persist():
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
        print("[+] Copied 'devinfo.img' to main directory for processing.")
    if persist_img_src.exists():
        shutil.copy(persist_img_src, persist_img)
        print("[+] Copied 'persist.img' to main directory for processing.")

    if not devinfo_img.exists() and not persist_img.exists():
        print("[!] Error: 'devinfo.img' and 'persist.img' both not found in main directory. Aborting.")
        raise FileNotFoundError("devinfo.img or persist.img not found for patching.")
        
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    backup_critical_dir = BASE_DIR / f"backup_critical_{timestamp}"
    backup_critical_dir.mkdir(exist_ok=True)
    
    print(f"[*] Backing up critical images to '{backup_critical_dir.name}'...")
    
    if devinfo_img.exists():
        shutil.copy(devinfo_img, backup_critical_dir)
        print(f"[+] Backed up '{devinfo_img.name}'.")
    if persist_img.exists():
        shutil.copy(persist_img, backup_critical_dir)
        print(f"[+] Backed up '{persist_img.name}'.")
    print("[+] Backup complete.\n")

    print(f"[*] Cleaning up old '{OUTPUT_DP_DIR.name}' folder...")
    if OUTPUT_DP_DIR.exists():
        shutil.rmtree(OUTPUT_DP_DIR)
    OUTPUT_DP_DIR.mkdir(exist_ok=True)

    target_code = "CN"
    
    while True:
        print(f"[*] Searching for target code '{target_code}XX' in images...")
        if imgpatch.check_target_exists(target_code):
            print(f"[+] Found target code '{target_code}XX'.")
            replacement_code = select_country_code("SELECT REPLACEMENT COUNTRY CODE")
            
            print("[*] Running patch script...")
            imgpatch.edit_devinfo_persist(target_code, replacement_code)
            break
        else:
            print(f"[!] Target code '{target_code}XX' not found in devinfo.img or persist.img.")
            choice = ""
            while choice not in ['y', 'n']:
                choice = input("    Manually select a new target code to search for? (y/n): ").lower().strip()
            
            if choice == 'n':
                print("[*] Skipping devinfo/persist patching.")
                devinfo_img.unlink(missing_ok=True)
                persist_img.unlink(missing_ok=True)
                return
            else:
                target_code = select_country_code("SELECT NEW TARGET COUNTRY CODE")

    modified_devinfo = BASE_DIR / "devinfo_modified.img"
    modified_persist = BASE_DIR / "persist_modified.img"
    
    if modified_devinfo.exists():
        shutil.move(modified_devinfo, OUTPUT_DP_DIR / "devinfo.img")
    if modified_persist.exists():
        shutil.move(modified_persist, OUTPUT_DP_DIR / "persist.img")
        
    print(f"\n[*] Final images have been moved to '{OUTPUT_DP_DIR.name}' folder.")
    
    print("[*] Cleaning up original image files...")
    devinfo_img.unlink(missing_ok=True)
    persist_img.unlink(missing_ok=True)
    
    print("\n" + "=" * 61)
    print("  SUCCESS!")
    print(f"  Modified images are ready in the '{OUTPUT_DP_DIR.name}' folder.")
    print("=" * 61)

def modify_xml(wipe=0):
    print("--- Starting XML Modification Process ---")
    
    print("--- Waiting for 'image' folder ---")
    prompt = (
        "[STEP 1] Please copy the entire 'image' folder from your\n"
        "         unpacked Lenovo RSA firmware into the main directory."
    )
    utils.wait_for_directory(IMAGE_DIR, prompt)

    if WORKING_DIR.exists():
        shutil.rmtree(WORKING_DIR)
    if OUTPUT_XML_DIR.exists():
        shutil.rmtree(OUTPUT_XML_DIR)
    
    WORKING_DIR.mkdir()
    OUTPUT_XML_DIR.mkdir(exist_ok=True)
    print(f"\n[*] Created temporary '{WORKING_DIR.name}' folder.")

    try:
        imgpatch.modify_xml_algo(wipe=wipe)
    except Exception as e:
        print(f"[!] Error during XML modification: {e}", file=sys.stderr)
        raise
    finally:
        if WORKING_DIR.exists():
            shutil.rmtree(WORKING_DIR)
        print(f"[*] Cleaned up temporary '{WORKING_DIR.name}' folder.")
    
    print("\n" + "=" * 61)
    print("  SUCCESS!")
    print(f"  Modified XML files are ready in the '{OUTPUT_XML_DIR.name}'.")
    print("  You can now run 'Flash EDL' (Menu 10).")
    print("=" * 61)

def disable_ota():
    print("--- Starting Disable OTA Process ---")
    
    print("\n" + "="*61)
    print("  STEP 1/2: Waiting for ADB Connection")
    print("="*61)
    try:
        device.wait_for_adb()
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

# --- EDL Actions ---

def read_edl():
    print("--- Starting EDL Read Process ---")
    
    device.reboot_to_edl()
    print("[*] Waiting for 10 seconds for device to enter EDL mode...")
    time.sleep(10)
    
    BACKUP_DIR.mkdir(exist_ok=True)
    devinfo_out = BACKUP_DIR / "devinfo.img"
    persist_out = BACKUP_DIR / "persist.img"

    print(f"--- Waiting for EDL Loader File ---")
    required_files = [EDL_LOADER_FILENAME]
    prompt = (
        f"[STEP 1] Place the EDL loader file ('{EDL_LOADER_FILENAME}')\n"
        f"         into the '{IMAGE_DIR.name}' folder to proceed."
    )
    utils.wait_for_files(IMAGE_DIR, required_files, prompt)
    print(f"[+] Loader file '{EDL_LOADER_FILE.name}' found in '{IMAGE_DIR.name}'.")

    device.wait_for_edl()
        
    print("\n[*] Attempting to read 'devinfo' partition...")
    try:
        device.edl_read_part(EDL_LOADER_FILE, "devinfo", devinfo_out)
        print(f"[+] Successfully read 'devinfo' to '{devinfo_out}'.")
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] Failed to read 'devinfo': {e}", file=sys.stderr)

    print("\n[*] Attempting to read 'persist' partition...")
    try:
        device.edl_read_part(EDL_LOADER_FILE, "persist", persist_out)
        print(f"[+] Successfully read 'persist' to '{persist_out}'.")
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] Failed to read 'persist': {e}", file=sys.stderr)

    print(f"\n--- EDL Read Process Finished ---")
    print(f"[*] Files have been saved to the '{BACKUP_DIR.name}' folder.")
    print(f"[*] You can now run 'Patch devinfo/persist' (Menu 3) to patch them.")


def write_edl(skip_reset=False, skip_reset_edl=False):
    print("--- Starting EDL Write Process ---")

    if not OUTPUT_DP_DIR.exists():
        print(f"[!] Error: Patched images folder '{OUTPUT_DP_DIR.name}' not found.", file=sys.stderr)
        print("[!] Please run 'Patch devinfo/persist' (Menu 3) first to generate the modified images.", file=sys.stderr)
        raise FileNotFoundError(f"{OUTPUT_DP_DIR.name} not found.")
    print(f"[+] Found patched images folder: '{OUTPUT_DP_DIR.name}'.")

    if not skip_reset_edl:
        print(f"--- Waiting for EDL Loader File ---")
        required_files = [EDL_LOADER_FILENAME]
        prompt = (
            f"[STEP 1] Place the EDL loader file ('{EDL_LOADER_FILENAME}')\n"
            f"         into the '{IMAGE_DIR.name}' folder to proceed."
        )
        IMAGE_DIR.mkdir(exist_ok=True) 
        utils.wait_for_files(IMAGE_DIR, required_files, prompt)
        print(f"[+] Loader file '{EDL_LOADER_FILE.name}' found in '{IMAGE_DIR.name}'.")

        device.wait_for_edl()

    patched_devinfo = OUTPUT_DP_DIR / "devinfo.img"
    patched_persist = OUTPUT_DP_DIR / "persist.img"

    if not patched_devinfo.exists() and not patched_persist.exists():
         print(f"[!] Error: Neither 'devinfo.img' nor 'persist.img' found inside '{OUTPUT_DP_DIR.name}'.", file=sys.stderr)
         raise FileNotFoundError(f"No patched images found in {OUTPUT_DP_DIR.name}.")

    commands_executed = False
    
    try:
        if patched_devinfo.exists():
            print(f"\n[*] Attempting to write 'devinfo' partition with '{patched_devinfo.name}'...")
            device.edl_write_part(EDL_LOADER_FILE, "devinfo", patched_devinfo)
            print("[+] Successfully wrote 'devinfo'.")
            commands_executed = True
        else:
            print(f"\n[*] 'devinfo.img' not found in '{OUTPUT_DP_DIR.name}'. Skipping write.")

        if patched_persist.exists():
            print(f"\n[*] Attempting to write 'persist' partition with '{patched_persist.name}'...")
            device.edl_write_part(EDL_LOADER_FILE, "persist", patched_persist)
            print("[+] Successfully wrote 'persist'.")
            commands_executed = True
        else:
            print(f"\n[*] 'persist.img' not found in '{OUTPUT_DP_DIR.name}'. Skipping write.")

        if commands_executed and not skip_reset:
            print("\n[*] Operations complete. Resetting device...")
            device.edl_reset(EDL_LOADER_FILE)
            print("[+] Device reset command sent.")
        elif skip_reset:
            print("\n[*] Operations complete. Skipping device reset as requested.")
        else:
            print("\n[!] No partitions were written. Skipping reset.")

    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] An error occurred during the EDL write/reset operation: {e}", file=sys.stderr)
        raise

    if not skip_reset:
        print("\n" + "="*61)
        print("  FRIENDLY REMINDER:")
        print("  Please ensure you have a safe backup of your original")
        print("  'devinfo.img' and 'persist.img' files before proceeding")
        print("  with any manual flashing operations.")
        print("="*61)

    print("\n--- EDL Write Process Finished ---")

def _compare_rollback_indices():
    print("\n--- [STEP 1] Dumping Current Firmware via EDL ---")
    INPUT_CURRENT_DIR.mkdir(exist_ok=True)
    boot_out = INPUT_CURRENT_DIR / "boot.img"
    vbmeta_out = INPUT_CURRENT_DIR / "vbmeta_system.img"

    print(f"--- Waiting for EDL Loader File ---")
    required_loader = [EDL_LOADER_FILENAME]
    loader_prompt = (
        f"[REQUIRED] Place the EDL loader file ('{EDL_LOADER_FILENAME}')\n"
        f"         into the '{IMAGE_DIR.name}' folder to dump current firmware."
    )
    utils.wait_for_files(IMAGE_DIR, required_loader, loader_prompt)
    print(f"[+] Loader file '{EDL_LOADER_FILE.name}' found in '{IMAGE_DIR.name}'.")

    device.wait_for_edl()
        
    print("\n[*] Attempting to read 'boot' partition...")
    try:
        device.edl_read_part(EDL_LOADER_FILE, "boot_a", boot_out)
        print(f"[+] Successfully read 'boot' to '{boot_out}'.")
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] Failed to read 'boot': {e}", file=sys.stderr)
        raise 

    print("\n[*] Attempting to read 'vbmeta_system' partition...")
    try:
        device.edl_read_part(EDL_LOADER_FILE, "vbmeta_system_a", vbmeta_out)
        print(f"[+] Successfully read 'vbmeta_system' to '{vbmeta_out}'.")
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] Failed to read 'vbmeta_system': {e}", file=sys.stderr)
        raise 
        
    print("\n--- [STEP 1] Dump complete ---")
    
    print("\n--- [STEP 2] Comparing Rollback Indices ---")
    print("\n[*] Extracting current ROM's rollback indices...")
    current_boot_rb = 0
    current_vbmeta_rb = 0
    try:
        current_boot_info = imgpatch.extract_image_avb_info(INPUT_CURRENT_DIR / "boot.img")
        current_boot_rb = int(current_boot_info.get('rollback', '0'))
        
        current_vbmeta_info = imgpatch.extract_image_avb_info(INPUT_CURRENT_DIR / "vbmeta_system.img")
        current_vbmeta_rb = int(current_vbmeta_info.get('rollback', '0'))
    except Exception as e:
        print(f"[!] Error reading current image info: {e}. Please check files.", file=sys.stderr)
        return 'ERROR', 0, 0

    print(f"  > Current ROM's Boot Index: {current_boot_rb}")
    print(f"  > Current ROM's VBMeta System Index: {current_vbmeta_rb}")

    print("\n[*] Extracting new ROM's rollback indices (from 'image' folder)...")
    new_boot_img = IMAGE_DIR / "boot.img"
    new_vbmeta_img = IMAGE_DIR / "vbmeta_system.img"

    if not new_boot_img.exists() or not new_vbmeta_img.exists():
        print(f"[!] Error: 'boot.img' or 'vbmeta_system.img' not found in '{IMAGE_DIR.name}' folder.")
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
        return 'ERROR', 0, 0

    print(f"  > New ROM's Boot Index: {new_boot_rb}")
    print(f"  > New ROM's VBMeta System Index: {new_vbmeta_rb}")

    if new_boot_rb < current_boot_rb or new_vbmeta_rb < current_vbmeta_rb:
        print("\n[!] Downgrade detected! Anti-Rollback patching is REQUIRED.")
        return 'NEEDS_PATCH', current_boot_rb, current_vbmeta_rb
    else:
        print("\n[+] Indices are same or higher. No Anti-Rollback patch needed.")
        return 'MATCH', 0, 0

def read_anti_rollback():
    print("--- Anti-Rollback Status Check ---")
    utils.check_dependencies()
    
    try:
        status, _, _ = _compare_rollback_indices()
        print(f"\n--- Status Check Complete: {status} ---")
    except Exception as e:
        print(f"\n[!] An error occurred during status check: {e}", file=sys.stderr)

def patch_anti_rollback():
    print("--- Anti-Rollback Patcher ---")
    utils.check_dependencies()

    if OUTPUT_ANTI_ROLLBACK_DIR.exists():
        shutil.rmtree(OUTPUT_ANTI_ROLLBACK_DIR)
    OUTPUT_ANTI_ROLLBACK_DIR.mkdir(exist_ok=True)
    
    try:
        status, current_boot_rb, current_vbmeta_rb = _compare_rollback_indices()

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

def write_anti_rollback(skip_reset=False):
    print("--- Starting Anti-Rollback Write Process ---")

    boot_img = OUTPUT_ANTI_ROLLBACK_DIR / "boot.img"
    vbmeta_img = OUTPUT_ANTI_ROLLBACK_DIR / "vbmeta_system.img"

    if not boot_img.exists() or not vbmeta_img.exists():
        print(f"[!] Error: Patched images not found in '{OUTPUT_ANTI_ROLLBACK_DIR.name}'.", file=sys.stderr)
        print("[!] Please run 'Patch Anti-Rollback' (Menu 7) first.", file=sys.stderr)
        raise FileNotFoundError(f"Patched images not found in {OUTPUT_ANTI_ROLLBACK_DIR.name}")
    print(f"[+] Found patched images folder: '{OUTPUT_ANTI_ROLLBACK_DIR.name}'.")

    if not skip_reset:
        print(f"--- Waiting for EDL Loader File ---")
        required_files = [EDL_LOADER_FILENAME]
        prompt = (
            f"[STEP 1] Place the EDL loader file ('{EDL_LOADER_FILENAME}')\n"
            f"         into the '{IMAGE_DIR.name}' folder to proceed."
        )
        IMAGE_DIR.mkdir(exist_ok=True) 
        utils.wait_for_files(IMAGE_DIR, required_files, prompt)
        print(f"[+] Loader file '{EDL_LOADER_FILE.name}' found in '{IMAGE_DIR.name}'.")

        device.wait_for_edl()
    
    try:
        print(f"\n[*] Attempting to write 'boot' partition...")
        device.edl_write_part(EDL_LOADER_FILE, "boot_a", boot_img)
        print("[+] Successfully wrote 'boot'.")

        print(f"\n[*] Attempting to write 'vbmeta_system' partition...")
        device.edl_write_part(EDL_LOADER_FILE, "vbmeta_system_a", vbmeta_img)
        print("[+] Successfully wrote 'vbmeta_system'.")

        if not skip_reset:
            print("\n[*] Operations complete. Resetting device...")
            device.edl_reset(EDL_LOADER_FILE)
            print("[+] Device reset command sent.")
        else:
            print("\n[*] Operations complete. Skipping device reset as requested.")

    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] An error occurred during the EDL write operation: {e}", file=sys.stderr)
        raise
    
    print("\n--- Anti-Rollback Write Process Finished ---")

def flash_edl(skip_reset=False, skip_reset_edl=False):
    print("--- Starting Full EDL Flash Process ---")
    
    if not IMAGE_DIR.is_dir() or not any(IMAGE_DIR.iterdir()):
        print(f"[!] Error: The '{IMAGE_DIR.name}' folder is missing or empty.")
        print("[!] Please run 'Modify XML for Update' (Menu 9) first.")
        raise FileNotFoundError(f"{IMAGE_DIR.name} is missing or empty.")
        
    loader_path = EDL_LOADER_FILE_IMAGE
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
        OUTPUT_ROOT_DIR, 
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
    
    if copied_count == 0:
        print("[*] No 'output*' folders found. Proceeding with files already in 'image' folder.")
    
    print("\n[*] Resetting device to EDL to ensure a clean state for fh_loader...")
    try:
        device.edl_reset(loader_path, mode="edl")
        print("[+] Device reset-to-EDL command sent. Waiting 5 seconds for re-enumeration...")
        time.sleep(5)
    except Exception as e:
        print(f"[!] Failed to reset device to EDL: {e}", file=sys.stderr)
        print("[!] Proceeding anyway, but may fail. If so, reboot to EDL manually.")
            
    port = device.wait_for_edl()
    if not port:
        print("[!] Failed to find EDL port after reset. Aborting.")
        raise SystemExit("EDL port not found")
    
    print("\n--- [STEP 1] Flashing main firmware via rawprogram (fh_loader) ---")

    raw_xmls = [f for f in IMAGE_DIR.glob("rawprogram*.xml") if f.name != "rawprogram0.xml"]
    patch_xmls = list(IMAGE_DIR.glob("patch*.xml"))
    
    if not raw_xmls or not patch_xmls:
        print(f"[!] Error: 'rawprogram*.xml' (excluding rawprogram0.xml) or 'patch*.xml' files not found in '{IMAGE_DIR.name}'.")
        print(f"[!] Cannot flash. Please run XML modification first.")
        raise FileNotFoundError(f"Missing essential XML flash files in {IMAGE_DIR.name}")
        
    try:
        device.edl_rawprogram(loader_path, "UFS", raw_xmls, patch_xmls, port)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] An error occurred during main flash: {e}", file=sys.stderr)
        print("[!] The device may be in an unstable state. Do not reboot manually.")
        raise
        
    print("\n--- [STEP 2] Flashing patched devinfo/persist ---")
    
    patched_devinfo = OUTPUT_DP_DIR / "devinfo.img"
    patched_persist = OUTPUT_DP_DIR / "persist.img"

    if not OUTPUT_DP_DIR.exists() or (not patched_devinfo.exists() and not patched_persist.exists()):
        print(f"[*] '{OUTPUT_DP_DIR.name}' not found or is empty. Skipping devinfo/persist flash.")
    else:
        print("[*] 'output_dp' folder found. Proceeding to flash devinfo/persist...")
        
        if not skip_reset_edl:
            print("\n[*] Resetting device back into EDL mode for devinfo/persist flash...")
            try:
                device.edl_reset(loader_path, mode="edl")
                print("[+] Device reset-to-EDL command sent.")
            except Exception as e:
                 print(f"[!] Failed to reset device to EDL: {e}", file=sys.stderr)
                 print("[!] Please manually reboot to EDL mode.")
            
            device.wait_for_edl() 
        
        write_edl(skip_reset=True, skip_reset_edl=True)

    print("\n--- [STEP 3] Flashing patched Anti-Rollback images ---")
    arb_boot = OUTPUT_ANTI_ROLLBACK_DIR / "boot.img"
    arb_vbmeta = OUTPUT_ANTI_ROLLBACK_DIR / "vbmeta_system.img"

    if not OUTPUT_ANTI_ROLLBACK_DIR.exists() or (not arb_boot.exists() and not arb_vbmeta.exists()):
        print(f"[*] '{OUTPUT_ANTI_ROLLBACK_DIR.name}' not found or is empty. Skipping Anti-Rollback flash.")
    else:
        print(f"[*] '{OUTPUT_ANTI_ROLLBACK_DIR.name}' found. Proceeding to flash Anti-Rollback images...")
        if skip_reset_edl:
             print("[*] Assuming device is still in EDL mode from previous step...")
        else:
            print("\n[!] CRITICAL: This flow is not intended to be run manually.")
            print("[!] Please use the 'Patch and Flash' (Menu 1) option.")
            
        write_anti_rollback(skip_reset=True)

    if not skip_reset:
        print("\n[*] Final step: Resetting device to system...")
        try:
            device.edl_reset(loader_path)
            print("[+] Device reset command sent.")
        except Exception as e:
             print(f"[!] Failed to reset device: {e}", file=sys.stderr)
    else:
        print("[*] Skipping final device reset as requested.")

    if not skip_reset:
        print("\n--- Full EDL Flash Process Finished ---")


def root_device():
    print("--- Starting Root Device Process ---")
    
    print(f"[*] Cleaning up old '{OUTPUT_ROOT_DIR.name}' and '{WORKING_BOOT_DIR.name}' folders...")
    if OUTPUT_ROOT_DIR.exists():
        shutil.rmtree(OUTPUT_ROOT_DIR)
    if WORKING_BOOT_DIR.exists():
        shutil.rmtree(WORKING_BOOT_DIR)
    
    OUTPUT_ROOT_DIR.mkdir(exist_ok=True)
    WORKING_BOOT_DIR.mkdir(exist_ok=True)
    BACKUP_BOOT_DIR.mkdir(exist_ok=True)

    utils.check_dependencies()
    
    magiskboot_exe = utils.get_platform_executable("magiskboot")
    fetch_exe = utils.get_platform_executable("fetch")
    
    if not fetch_exe.exists():
         print(f"[!] '{fetch_exe.name}' not found. Please run install.bat")
         sys.exit(1)
    downloader._ensure_magiskboot(fetch_exe, magiskboot_exe)

    print("\n--- [STEP 1/6] Waiting for ADB Connection ---")
    device.wait_for_adb()
    
    print("\n--- [STEP 2/6] Rebooting to EDL Mode ---")
    device.reboot_to_edl()
    print("[*] Waiting for 10 seconds for device to enter EDL mode...")
    time.sleep(10)

    print(f"--- Waiting for EDL Loader File ---")
    required_files = [EDL_LOADER_FILENAME]
    prompt = (
        f"[STEP 1] Place the EDL loader file ('{EDL_LOADER_FILENAME}')\n"
        f"         into the '{IMAGE_DIR.name}' folder to proceed."
    )
    utils.wait_for_files(IMAGE_DIR, required_files, prompt)
    print(f"[+] Loader file '{EDL_LOADER_FILE.name}' found in '{IMAGE_DIR.name}'.")

    device.wait_for_edl()

    print("\n--- [STEP 3/6] Dumping boot_a partition ---")
    dumped_boot_img = WORKING_BOOT_DIR / "boot.img"
    backup_boot_img = BACKUP_BOOT_DIR / "boot.img"
    base_boot_bak = BASE_DIR / "boot.bak.img"

    try:
        device.edl_read_part(EDL_LOADER_FILE, "boot_a", dumped_boot_img)
        print(f"[+] Successfully read 'boot_a' to '{dumped_boot_img}'.")
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] Failed to read 'boot_a': {e}", file=sys.stderr)
        raise

    print(f"[*] Backing up original boot.img to '{backup_boot_img.parent.name}' folder...")
    shutil.copy(dumped_boot_img, backup_boot_img)
    print(f"[*] Creating temporary backup for AVB processing...")
    shutil.copy(dumped_boot_img, base_boot_bak)
    print("[+] Backups complete.")

    print("\n--- [STEP 4/6] Patching dumped boot.img ---")
    patched_boot_path = imgpatch.patch_boot_with_root_algo(WORKING_BOOT_DIR, magiskboot_exe, fetch_exe)

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

    final_boot_img = OUTPUT_ROOT_DIR / "boot.img"
    shutil.move(patched_boot_path, final_boot_img)
    print(f"[+] Patched boot image saved to '{final_boot_img.parent.name}' folder.")

    print("\n--- [STEP 6/6] Flashing patched boot.img to boot_a ---")
    try:
        device.edl_write_part(EDL_LOADER_FILE, "boot_a", final_boot_img)
        print("[+] Successfully wrote patched 'boot.img' to 'boot_a'.")
        
        print("\n[*] Operations complete. Resetting device...")
        device.edl_reset(EDL_LOADER_FILE)
        print("[+] Device reset command sent.")
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] An error occurred during the EDL write/reset operation: {e}", file=sys.stderr)
        raise
    finally:
        base_boot_bak.unlink(missing_ok=True)

    print("\n--- Root Device Process Finished ---")


def unroot_device():
    print("--- Starting Unroot Device Process ---")
    
    backup_boot_file = BACKUP_BOOT_DIR / "boot.img"
    BACKUP_BOOT_DIR.mkdir(exist_ok=True)

    print("\n--- [STEP 1/5] Waiting for ADB Connection ---")
    device.wait_for_adb()
    
    print("\n--- [STEP 2/5] Rebooting to EDL Mode ---")
    device.reboot_to_edl()
    print("[*] Waiting for 10 seconds for device to enter EDL mode...")
    time.sleep(10)

    print(f"--- Waiting for EDL Loader File ---")
    required_files = [EDL_LOADER_FILENAME]
    prompt = (
        f"[STEP 1] Place the EDL loader file ('{EDL_LOADER_FILENAME}')\n"
        f"         into the '{IMAGE_DIR.name}' folder to proceed."
    )
    utils.wait_for_files(IMAGE_DIR, required_files, prompt)
    print(f"[+] Loader file '{EDL_LOADER_FILE.name}' found in '{IMAGE_DIR.name}'.")

    device.wait_for_edl()
    
    print("\n--- [STEP 3/5] Checking for backup boot.img ---")
    if not backup_boot_file.exists():
        prompt = (
            "[!] Backup file 'boot.img' not found.\n"
            f"    Please place your stock 'boot.img' (from your current firmware)\n"
            f"    into the '{BACKUP_BOOT_DIR.name}' folder."
        )
        utils.wait_for_files(BACKUP_BOOT_DIR, ["boot.img"], prompt)
    
    print("[+] Stock backup 'boot.img' found.")

    print("\n--- [STEP 4/5] Flashing stock boot.img to boot_a ---")
    try:
        device.edl_write_part(EDL_LOADER_FILE, "boot_a", backup_boot_file)
        print("[+] Successfully wrote stock 'boot.img' to 'boot_a'.")
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] An error occurred during the EDL write operation: {e}", file=sys.stderr)
        raise

    print("\n--- [STEP 5/5] Resetting device ---")
    try:
        device.edl_reset(EDL_LOADER_FILE)
        print("[+] Device reset command sent.")
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] An error occurred during the EDL reset operation: {e}", file=sys.stderr)
        raise

    print("\n--- Unroot Device Process Finished ---")