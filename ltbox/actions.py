import os
import platform
import re
import shutil
import subprocess
import sys
from datetime import datetime
from pathlib import Path

from ltbox.constants import *
from ltbox import utils

# --- Core Functions ---
def patch_boot_with_root():
    print("--- Starting boot.img patching process ---")
    magiskboot_exe = utils.get_platform_executable("magiskboot")
    fetch_exe = utils.get_platform_executable("fetch")
    
    patched_boot_path = BASE_DIR / "boot.root.img"

    if not fetch_exe.exists():
         print(f"[!] '{fetch_exe.name}' not found. Please run install.bat")
         sys.exit(1)

    utils._ensure_magiskboot(fetch_exe, magiskboot_exe)

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

    original_cwd = Path.cwd()
    os.chdir(WORK_DIR)

    try:
        shutil.copy(boot_img, WORK_DIR)

        print("\n[1/8] Unpacking boot image...")
        utils.run_command([str(magiskboot_exe), "unpack", "boot.img"])
        if not (WORK_DIR / "kernel").exists():
            print("[!] Failed to unpack boot.img. The image might be invalid.")
            sys.exit(1)
        print("[+] Unpack successful.")

        print("\n[2/8] Verifying kernel version...")
        result = utils.run_command([str(PYTHON_EXE), str(GET_KERNEL_VER_PY), "kernel"])
        target_kernel_version = result.stdout.strip()

        if not re.match(r"\d+\.\d+\.\d+", target_kernel_version):
             print(f"[!] Invalid kernel version returned from script: '{target_kernel_version}'")
             sys.exit(1)
        
        print(f"[+] Target kernel version for download: {target_kernel_version}")

        kernel_image_path = utils._get_gki_kernel(fetch_exe, target_kernel_version, WORK_DIR)

        print("\n[5/8] Replacing original kernel with the new one...")
        shutil.move(str(kernel_image_path), "kernel")
        print("[+] Kernel replaced.")

        print("\n[6/8] Repacking boot image...")
        utils.run_command([str(magiskboot_exe), "repack", "boot.img"])
        if not (WORK_DIR / "new-boot.img").exists():
            print("[!] Failed to repack the boot image.")
            sys.exit(1)
        shutil.move("new-boot.img", patched_boot_path)
        print("[+] Repack successful.")

        utils._download_ksu_apk(fetch_exe, BASE_DIR)

    finally:
        os.chdir(original_cwd)
        if WORK_DIR.exists():
            shutil.rmtree(WORK_DIR)
        if boot_img.exists():
            boot_img.unlink()
        print("\n--- Cleaning up ---")

    if patched_boot_path.exists():
        return patched_boot_path
    return None

def convert_images():
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
    utils.run_command([str(PYTHON_EXE), str(EDIT_IMAGES_PY), "vndrboot", str(vendor_boot_bak)])

    vendor_boot_prc = BASE_DIR / "vendor_boot_prc.img"
    print("\n[*] Verifying conversion result...")
    if not vendor_boot_prc.exists():
        print("[!] 'vendor_boot_prc.img' was not created. No changes made.")
        raise FileNotFoundError("vendor_boot_prc.img not created")
    print("[+] Conversion to PRC successful.\n")

    print("--- Extracting image information ---")
    vbmeta_info = utils.extract_image_avb_info(vbmeta_bak)
    vendor_boot_info = utils.extract_image_avb_info(vendor_boot_bak)
    print("[+] Information extracted.\n")
    
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

    finalize_images()

def finalize_images():
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

    patched_boot_path = patch_boot_with_root()

    if patched_boot_path and patched_boot_path.exists():
        print("\n--- Finalizing ---")
        final_boot_img = OUTPUT_ROOT_DIR / "boot.img"
        
        utils.process_boot_image(patched_boot_path)

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

    print("[*] Running patch script...")
    utils.run_command([str(PYTHON_EXE), str(EDIT_IMAGES_PY), "dp"])

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


def read_edl():
    print("--- Starting EDL Read Process ---")
    
    edl_ng_exe = utils._ensure_edl_ng()
    
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

    utils.wait_for_edl()
        
    print("\n[*] Attempting to read 'devinfo' partition...")
    try:
        utils.run_command([
            str(edl_ng_exe),
            "--loader", str(EDL_LOADER_FILE), 
            "read-part", "devinfo", str(devinfo_out) 
        ])
        print(f"[+] Successfully read 'devinfo' to '{devinfo_out}'.")
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] Failed to read 'devinfo': {e}", file=sys.stderr)

    print("\n[*] Attempting to read 'persist' partition...")
    try:
        utils.run_command([
            str(edl_ng_exe),
            "--loader", str(EDL_LOADER_FILE), 
            "read-part", "persist", str(persist_out) 
        ])
        print(f"[+] Successfully read 'persist' to '{persist_out}'.")
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] Failed to read 'persist': {e}", file=sys.stderr)

    print(f"\n--- EDL Read Process Finished ---")
    print(f"[*] Files have been saved to the '{BACKUP_DIR.name}' folder.")
    print(f"[*] You can now run 'Patch devinfo/persist' (Menu 3) to patch them.")


def write_edl(skip_reset=False, skip_reset_edl=False):
    print("--- Starting EDL Write Process ---")

    edl_ng_exe = utils._ensure_edl_ng()

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

        utils.wait_for_edl()

    patched_devinfo = OUTPUT_DP_DIR / "devinfo.img"
    patched_persist = OUTPUT_DP_DIR / "persist.img"

    if not patched_devinfo.exists() and not patched_persist.exists():
         print(f"[!] Error: Neither 'devinfo.img' nor 'persist.img' found inside '{OUTPUT_DP_DIR.name}'.", file=sys.stderr)
         raise FileNotFoundError(f"No patched images found in {OUTPUT_DP_DIR.name}.")

    commands_executed = False
    
    try:
        if patched_devinfo.exists():
            print(f"\n[*] Attempting to write 'devinfo' partition with '{patched_devinfo.name}'...")
            utils.run_command([
                str(edl_ng_exe),
                "--loader", str(EDL_LOADER_FILE), 
                "write-part", "devinfo", str(patched_devinfo)
            ])
            print("[+] Successfully wrote 'devinfo'.")
            commands_executed = True
        else:
            print(f"\n[*] 'devinfo.img' not found in '{OUTPUT_DP_DIR.name}'. Skipping write.")

        if patched_persist.exists():
            print(f"\n[*] Attempting to write 'persist' partition with '{patched_persist.name}'...")
            utils.run_command([
                str(edl_ng_exe),
                "--loader", str(EDL_LOADER_FILE), 
                "write-part", "persist", str(patched_persist)
            ])
            print("[+] Successfully wrote 'persist'.")
            commands_executed = True
        else:
            print(f"\n[*] 'persist.img' not found in '{OUTPUT_DP_DIR.name}'. Skipping write.")

        if commands_executed and not skip_reset:
            print("\n[*] Operations complete. Resetting device...")
            utils.run_command([
                str(edl_ng_exe),
                "--loader", str(EDL_LOADER_FILE), 
                "reset"
            ])
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

def _compare_rollback_indices(edl_ng_exe):
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

    utils.wait_for_edl()
        
    print("\n[*] Attempting to read 'boot' partition...")
    try:
        utils.run_command([
            str(edl_ng_exe),
            "--loader", str(EDL_LOADER_FILE), 
            "read-part", "boot", str(boot_out) 
        ])
        print(f"[+] Successfully read 'boot' to '{boot_out}'.")
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] Failed to read 'boot': {e}", file=sys.stderr)
        raise 

    print("\n[*] Attempting to read 'vbmeta_system' partition...")
    try:
        utils.run_command([
            str(edl_ng_exe),
            "--loader", str(EDL_LOADER_FILE), 
            "read-part", "vbmeta_system", str(vbmeta_out) 
        ])
        print(f"[+] Successfully read 'vbmeta_system' to '{vbmeta_out}'.")
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] Failed to read 'vbmeta_system': {e}", file=sys.stderr)
        raise 
        
    print("\n--- [STEP 1] Dump complete ---")
    
    print("\n--- [STEP 2] Comparing Rollback Indices ---")
    print("\n[*] Extracting current firmware rollback indices...")
    current_boot_rb = 0
    current_vbmeta_rb = 0
    try:
        current_boot_info = utils.extract_image_avb_info(INPUT_CURRENT_DIR / "boot.img")
        current_boot_rb = int(current_boot_info.get('rollback', '0'))
        
        current_vbmeta_info = utils.extract_image_avb_info(INPUT_CURRENT_DIR / "vbmeta_system.img")
        current_vbmeta_rb = int(current_vbmeta_info.get('rollback', '0'))
    except Exception as e:
        print(f"[!] Error reading current image info: {e}. Please check files.", file=sys.stderr)
        return 'ERROR', 0, 0

    print(f"  > Current Boot Index: {current_boot_rb}")
    print(f"  > Current VBMeta System Index: {current_vbmeta_rb}")

    print("\n[*] Extracting new firmware rollback indices (from 'image' folder)...")
    new_boot_img = IMAGE_DIR / "boot.img"
    new_vbmeta_img = IMAGE_DIR / "vbmeta_system.img"

    if not new_boot_img.exists() or not new_vbmeta_img.exists():
        print(f"[!] Error: 'boot.img' or 'vbmeta_system.img' not found in '{IMAGE_DIR.name}' folder.")
        return 'MISSING_NEW', 0, 0
        
    new_boot_rb = 0
    new_vbmeta_rb = 0
    try:
        new_boot_info = utils.extract_image_avb_info(new_boot_img)
        new_boot_rb = int(new_boot_info.get('rollback', '0'))
        
        new_vbmeta_info = utils.extract_image_avb_info(new_vbmeta_img)
        new_vbmeta_rb = int(new_vbmeta_info.get('rollback', '0'))
    except Exception as e:
        print(f"[!] Error reading new image info: {e}. Please check files.", file=sys.stderr)
        return 'ERROR', 0, 0

    print(f"  > New Boot Index: {new_boot_rb}")
    print(f"  > New VBMeta System Index: {new_vbmeta_rb}")

    if new_boot_rb < current_boot_rb or new_vbmeta_rb < current_vbmeta_rb:
        print("\n[!] Downgrade detected! Anti-Rollback patching is REQUIRED.")
        return 'NEEDS_PATCH', current_boot_rb, current_vbmeta_rb
    else:
        print("\n[+] Indices are same or higher. No Anti-Rollback patch needed.")
        return 'MATCH', 0, 0

def read_anti_rollback():
    print("--- Anti-Rollback Status Check ---")
    utils.check_dependencies()
    edl_ng_exe = utils._ensure_edl_ng()
    
    try:
        status, _, _ = _compare_rollback_indices(edl_ng_exe)
        print(f"\n--- Status Check Complete: {status} ---")
    except Exception as e:
        print(f"\n[!] An error occurred during status check: {e}", file=sys.stderr)

def patch_anti_rollback():
    print("--- Anti-Rollback Patcher ---")
    utils.check_dependencies()
    edl_ng_exe = utils._ensure_edl_ng()

    if OUTPUT_ANTI_ROLLBACK_DIR.exists():
        shutil.rmtree(OUTPUT_ANTI_ROLLBACK_DIR)
    OUTPUT_ANTI_ROLLBACK_DIR.mkdir(exist_ok=True)
    
    try:
        status, current_boot_rb, current_vbmeta_rb = _compare_rollback_indices(edl_ng_exe)

        if status != 'NEEDS_PATCH':
            print("\n[!] No patching is required or files are missing. Aborting patch.")
            return

        print("\n--- [STEP 3] Patching New Firmware ---")
        
        utils.patch_chained_image_rollback(
            image_name="boot.img",
            current_rb_index=current_boot_rb,
            new_image_path=(IMAGE_DIR / "boot.img"),
            patched_image_path=(OUTPUT_ANTI_ROLLBACK_DIR / "boot.img")
        )
        
        print("-" * 20)
        
        utils.patch_vbmeta_image_rollback(
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

    edl_ng_exe = utils._ensure_edl_ng()

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

        utils.wait_for_edl()
    
    try:
        print(f"\n[*] Attempting to write 'boot' partition...")
        utils.run_command([
            str(edl_ng_exe),
            "--loader", str(EDL_LOADER_FILE), 
            "write-part", "boot", str(boot_img)
        ])
        print("[+] Successfully wrote 'boot'.")

        print(f"\n[*] Attempting to write 'vbmeta_system' partition...")
        utils.run_command([
            str(edl_ng_exe),
            "--loader", str(EDL_LOADER_FILE), 
            "write-part", "vbmeta_system", str(vbmeta_img)
        ])
        print("[+] Successfully wrote 'vbmeta_system'.")

        if not skip_reset:
            print("\n[*] Operations complete. Resetting device...")
            utils.run_command([
                str(edl_ng_exe),
                "--loader", str(EDL_LOADER_FILE), 
                "reset"
            ])
            print("[+] Device reset command sent.")
        else:
            print("\n[*] Operations complete. Skipping device reset as requested.")

    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"[!] An error occurred during the EDL write operation: {e}", file=sys.stderr)
        raise
    
    print("\n--- Anti-Rollback Write Process Finished ---")


def clean_workspace():
    print("--- Starting Cleanup Process ---")
    print("This will remove all input/output folders and downloaded tools.")
    print("The 'python3' and 'backup' folders will NOT be removed.")
    print("-" * 50)

    folders_to_remove = [
        INPUT_CURRENT_DIR, INPUT_NEW_DIR,
        OUTPUT_DIR, OUTPUT_ROOT_DIR, OUTPUT_DP_DIR, OUTPUT_ANTI_ROLLBACK_DIR,
        WORK_DIR,
        AVB_DIR,
        IMAGE_DIR,
        WORKING_DIR,
        OUTPUT_XML_DIR
    ]
    
    print("[*] Removing directories...")
    for folder in folders_to_remove:
        if folder.exists():
            try:
                shutil.rmtree(folder)
                print(f"  > Removed: {folder.name}{os.sep}")
            except OSError as e:
                print(f"[!] Error removing {folder.name}: {e}", file=sys.stderr)
        else:
            print(f"  > Skipping (not found): {folder.name}{os.sep}")

    print("\n[*] Removing downloaded tools from 'tools' folder...")
    tools_files_to_remove = [
        "fetch.exe", "fetch-linux", "fetch-macos",
        "magiskboot.exe", "magiskboot-linux", "magiskboot-macos",
        "edl-ng.exe",
        "magiskboot-*.zip",
        "edl-ng-*.zip"
    ]
    
    cleaned_tools_files = 0
    for pattern in tools_files_to_remove:
        for f in TOOLS_DIR.glob(pattern):
            try:
                f.unlink()
                print(f"  > Removed tool: {f.name}")
                cleaned_tools_files += 1
            except OSError as e:
                print(f"[!] Error removing {f.name}: {e}", file=sys.stderr)
    
    if cleaned_tools_files == 0:
        print("  > No downloaded tools found to clean.")

    print("\n[*] Cleaning up temporary files from root directory...")
    file_patterns_to_remove = [
        "*.bak.img",
        "*.root.img",
        "*prc.img",
        "*modified.img",
        "image_info_*.txt",
        "KernelSU*.apk",
        "devinfo.img", 
        "persist.img", 
        "boot.img", 
        "vbmeta.img" 
    ]
    
    cleaned_root_files = 0
    for pattern in file_patterns_to_remove:
        for f in BASE_DIR.glob(pattern):
            try:
                f.unlink()
                print(f"  > Removed: {f.name}")
                cleaned_root_files += 1
            except OSError as e:
                print(f"[!] Error removing {f.name}: {e}", file=sys.stderr)
    
    if cleaned_root_files == 0:
        print("  > No temporary files found to clean.")

    print("\n--- Cleanup Finished ---")

def modify_xml(wipe=0):
    print("--- Starting XML Modification Process ---")

    if not DECRYPT_PY.exists():
        print(f"[!] Error: Decryption script not found at '{DECRYPT_PY}'")
        print("[!] Please ensure 'decrypt_x.py' is in the 'tools' folder.")
        sys.exit(1)
        
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
    print(f"\n[*] Created temporary '{WORKING_DIR.name}' folder.")

    print("[*] Decrypting *.x files and moving to 'working' folder...")
    xml_files = []
    for file in IMAGE_DIR.glob("*.x"):
        out_file = WORKING_DIR / file.with_suffix('.xml').name
        try:
            utils.run_command([str(PYTHON_EXE), str(DECRYPT_PY), str(file), str(out_file)])
            print(f"  > Decrypted: {file.name} -> {out_file.name}")
            xml_files.append(out_file)
        except Exception as e:
            print(f"[!] Failed to decrypt {file.name}: {e}", file=sys.stderr)
            
    if not xml_files:
        print(f"[!] No '*.x' files found in '{IMAGE_DIR.name}'. Aborting.")
        shutil.rmtree(WORKING_DIR)
        raise FileNotFoundError(f"No *.x files found in {IMAGE_DIR.name}")

    contents_xml = WORKING_DIR / "contents.xml"
    if not contents_xml.exists():
        print(f"[!] Error: 'contents.xml' not found in '{WORKING_DIR.name}'.")
        print("[!] This file is essential for the flashing process. Aborting.")
        shutil.rmtree(WORKING_DIR)
        raise FileNotFoundError(f"contents.xml not found in {WORKING_DIR.name}")

    rawprogram4 = WORKING_DIR / "rawprogram4.xml"
    rawprogram_unsparse4 = WORKING_DIR / "rawprogram_unsparse4.xml"
    if not rawprogram4.exists() and rawprogram_unsparse4.exists():
        print(f"[*] 'rawprogram4.xml' not found. Copying 'rawprogram_unsparse4.xml'...")
        shutil.copy(rawprogram_unsparse4, rawprogram4)

    print("\n[*] Modifying 'rawprogram_save_persist_unsparse0.xml'...")
    rawprogram_save = WORKING_DIR / "rawprogram_save_persist_unsparse0.xml"
    if rawprogram_save.exists():
        try:
            with open(rawprogram_save, 'r', encoding='utf-8') as f:
                content = f.read()
            
            if wipe == 1:
                print(f"  > [WIPE] Removing metadata and userdata entries...")
                for i in range(1, 11):
                    content = content.replace(f'filename="metadata_{i}.img"', '')
                for i in range(1, 21):
                    content = content.replace(f'filename="userdata_{i}.img"', '')
            else:
                print(f"  > [NO WIPE] Skipping metadata and userdata removal.")
                
            with open(rawprogram_save, 'w', encoding='utf-8') as f:
                f.write(content)
            print("  > Patched 'rawprogram_save_persist_unsparse0.xml' successfully.")
        except Exception as e:
            print(f"[!] Error patching 'rawprogram_save_persist_unsparse0.xml': {e}", file=sys.stderr)
    else:
        print("  > 'rawprogram_save_persist_unsparse0.xml' not found. Skipping.")

    print("\n[*] Modifying 'rawprogram4.xml'...")
    if rawprogram4.exists():
        try:
            with open(rawprogram4, 'r', encoding='utf-8') as f:
                content = f.read()
            
            if not any(IMAGE_DIR.glob("vm-bootsys*.img")):
                print("  > 'vm-bootsys' image not found. Removing from XML...")
                content = content.replace('filename="vm-bootsys.img"', '')
            else:
                print("  > 'vm-bootsys' image found. Keeping in XML.")

            if not any(IMAGE_DIR.glob("vm-persist*.img")):
                print("  > 'vm-persist' image not found. Removing from XML...")
                content = content.replace('filename="vm-persist.img"', '')
            else:
                print("  > 'vm-persist' image found. Keeping in XML.")

            with open(rawprogram4, 'w', encoding='utf-8') as f:
                f.write(content)
            print("  > Patched 'rawprogram4.xml' successfully.")
        except Exception as e:
            print(f"[!] Error patching 'rawprogram4.xml': {e}", file=sys.stderr)
    else:
        print("  > 'rawprogram4.xml' not found. Skipping.")

    print("\n[*] Deleting unnecessary XML files...")
    files_to_delete = [
        WORKING_DIR / "rawprogram_unsparse0.xml",
        WORKING_DIR / "contents.xml",
        *WORKING_DIR.glob("*_WIPE_PARTITIONS.xml"),
        *WORKING_DIR.glob("*_BLANK_GPT.xml")
    ]
    for f in files_to_delete:
        if f.exists():
            f.unlink()
            print(f"  > Deleted: {f.name}")

    OUTPUT_XML_DIR.mkdir(exist_ok=True)
    print(f"\n[*] Moving modified XML files to '{OUTPUT_XML_DIR.name}'...")
    moved_count = 0
    for f in WORKING_DIR.glob("*.xml"):
        shutil.move(str(f), OUTPUT_XML_DIR / f.name)
        moved_count += 1
        
    print(f"[+] Moved {moved_count} modified XML file(s).")
    
    shutil.rmtree(WORKING_DIR)
    print(f"[*] Cleaned up temporary '{WORKING_DIR.name}' folder.")
    
    print("\n" + "=" * 61)
    print("  SUCCESS!")
    print(f"  Modified XML files are ready in the '{OUTPUT_XML_DIR.name}'.")
    print("  You can now run 'Flash EDL' (Menu 10).")
    print("=" * 61)

def flash_edl(skip_reset=False, skip_reset_edl=False):
    print("--- Starting Full EDL Flash Process ---")
    
    edl_ng_exe = utils._ensure_edl_ng()

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
    
    utils.wait_for_edl()
    
    print("\n--- [STEP 1] Flashing main firmware via rawprogram ---")
    raw_xmls = list(IMAGE_DIR.glob("rawprogram*.xml"))
    patch_xmls = list(IMAGE_DIR.glob("patch*.xml"))
    
    if not raw_xmls or not patch_xmls:
        print(f"[!] Error: 'rawprogram*.xml' or 'patch*.xml' files not found in '{IMAGE_DIR.name}'.")
        print(f"[!] Cannot flash. Please run XML modification first.")
        raise FileNotFoundError(f"Missing essential XML flash files in {IMAGE_DIR.name}")
        
    cmd = [
        str(edl_ng_exe), 
        "--loader", str(loader_path), 
        "--memory", "UFS", 
        "rawprogram", 
        *[str(p) for p in raw_xmls], 
        *[str(p) for p in patch_xmls]
    ]
    
    try:
        utils.run_command(cmd)
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
                utils.run_command([
                    str(edl_ng_exe),
                    "--loader", str(loader_path), 
                    "reset", "edl"
                ])
                print("[+] Device reset-to-EDL command sent.")
            except Exception as e:
                 print(f"[!] Failed to reset device to EDL: {e}", file=sys.stderr)
                 print("[!] Please manually reboot to EDL mode.")
            
            utils.wait_for_edl() 
        
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
            utils.run_command([
                str(edl_ng_exe),
                "--loader", str(loader_path), 
                "reset"
            ])
            print("[+] Device reset command sent.")
        except Exception as e:
             print(f"[!] Failed to reset device: {e}", file=sys.stderr)
    else:
        print("[*] Skipping final device reset as requested.")

    if not skip_reset:
        print("\n--- Full EDL Flash Process Finished ---")

def patch_all(wipe=0):
    if wipe == 1:
        print("--- [WIPE MODE] Starting Automated Install & Flash ROW ROM Process ---")
    else:
        print("--- [NO WIPE MODE] Starting Automated Update & Flash ROW ROM Process ---")
    
    print("\n--- [STEP 1/7] Waiting for RSA Firmware 'image' folder ---")
    prompt = (
        "Please copy the entire 'image' folder from your\n"
        "         unpacked Lenovo RSA firmware into the main directory.\n"
        r"         (Typical Location: C:\ProgramData\RSA\Download\RomFiles\...)"
    )
    utils.wait_for_directory(IMAGE_DIR, prompt)
    print("[+] 'image' folder found.")
    
    try:
        print("\n" + "="*61)
        print("  STEP 2/7: Converting ROM (PRC to ROW)")
        print("="*61)
        convert_images()
        print("\n--- [STEP 2/7] ROM Conversion SUCCESS ---")

        print("\n" + "="*61)
        print("  STEP 3/7: Modifying XML Files")
        print("="*61)
        modify_xml(wipe=wipe)
        print("\n--- [STEP 3/7] XML Modification SUCCESS ---")
        
        print("\n" + "="*61)
        print("  STEP 4/7: Dumping devinfo/persist for patching")
        print("="*61)
        read_edl()
        print("\n--- [STEP 4/7] Dump SUCCESS ---")
        
        print("\n" + "="*61)
        print("  STEP 5/7: Patching devinfo/persist")
        print("="*61)
        edit_devinfo_persist()
        print("\n--- [STEP 5/7] Patching SUCCESS ---")
        
        print("\n" + "="*61)
        print("  STEP 6/7: Checking and Patching Anti-Rollback")
        print("="*61)
        read_anti_rollback()
        patch_anti_rollback()
        print("\n--- [STEP 6/7] Anti-Rollback Check/Patch SUCCESS ---")
        
        print("\n" + "="*61)
        print("  [FINAL STEP] Flashing All Images via EDL")
        print("="*61)
        print("The device will now be flashed with all modified images.")
        flash_edl(skip_reset_edl=True) 
        
        print("\n" + "=" * 61)
        print("  FULL PROCESS COMPLETE!")
        print("  Your device should now reboot with a patched ROW ROM.")
        print("=" * 61)

    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
        print("\n" + "!" * 61)
        print("  AN ERROR OCCURRED: Process Halted.")
        print(f"  Error details: {e}")
        print("!" * 61)
        sys.exit(1)
    except SystemExit:
        print("\n" + "!" * 61)
        print("  PROCESS HALTED BY SCRIPT.")
        print("!" * 61)
    except KeyboardInterrupt:
        print("\n" + "!" * 61)
        print("  PROCESS CANCELLED BY USER.")
        print("!" * 61)