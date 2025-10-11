import argparse
import os
import platform
import re
import shutil
import subprocess
import sys
import zipfile
from datetime import datetime
from pathlib import Path

import requests

# --- Constants ---
BASE_DIR = Path(__file__).parent.resolve()
TOOLS_DIR = BASE_DIR / "tools"
PYTHON_DIR = BASE_DIR / "python3"
AVB_DIR = TOOLS_DIR / "avb"
OUTPUT_DIR = BASE_DIR / "output"
OUTPUT_ROOT_DIR = BASE_DIR / "output_root"
OUTPUT_DP_DIR = BASE_DIR / "output_dp"
BACKUP_DIR = BASE_DIR / "backup"
WORK_DIR = BASE_DIR / "patch_work"

PYTHON_EXE = PYTHON_DIR / "python.exe"
AVBTOOL_PY = AVB_DIR / "avbtool.py"
EDIT_IMAGES_PY = TOOLS_DIR / "edit_images.py"
GET_KERNEL_VER_PY = TOOLS_DIR / "get_kernel_ver.py"

KSU_APK_REPO = "KernelSU-Next/KernelSU-Next"
KSU_APK_TAG = "v1.1.1"

RELEASE_OWNER = "WildKernels"
RELEASE_REPO = "GKI_KernelSU_SUSFS"
RELEASE_TAG = "v1.5.9-r36"
REPO_URL = f"https://github.com/{RELEASE_OWNER}/{RELEASE_REPO}"

ANYKERNEL_ZIP_FILENAME = "AnyKernel3.zip"


# --- Helper Functions ---
def run_command(command, shell=False, check=True, env=None, capture=False):
    """Executes a command and handles its output."""
    env = env or os.environ.copy()
    env['PATH'] = str(TOOLS_DIR) + os.pathsep + env['PATH']

    try:
        process = subprocess.run(
            command, shell=shell, check=check, capture_output=True,
            text=True, encoding='utf-8', errors='ignore', env=env
        )
        if not capture:
            if process.stdout:
                print(process.stdout.strip())
            if process.stderr:
                print(process.stderr.strip(), file=sys.stderr)
        return process
    except FileNotFoundError as e:
        print(f"Error: Command not found - {e.filename}", file=sys.stderr)
        raise
    except subprocess.CalledProcessError as e:
        print(f"Error executing command: {' '.join(map(str, command))}", file=sys.stderr)
        print(f"Return code: {e.returncode}", file=sys.stderr)
        if e.stdout:
            print(f"Stdout:\n{e.stdout.strip()}", file=sys.stderr)
        if e.stderr:
            print(f"Stderr:\n{e.stderr.strip()}", file=sys.stderr)
        raise

def get_platform_executable(name):
    """Returns the path to a platform-specific executable."""
    system = platform.system()
    executables = {
        "Windows": f"{name}.exe",
        "Linux": f"{name}-linux",
        "Darwin": f"{name}-macos"
    }
    exe_name = executables.get(system)
    if not exe_name:
        raise RuntimeError(f"Unsupported operating system: {system}")
    return TOOLS_DIR / exe_name

def check_dependencies():
    """Checks for required files and exits if any are missing."""
    print("--- Checking for required files ---")
    dependencies = {
        "Python Environment": PYTHON_EXE,
        "RSA4096 Key": AVB_DIR / "testkey_rsa4096.pem",
        "RSA2048 Key": AVB_DIR / "testkey_rsa2048.pem",
        "avbtool": AVBTOOL_PY,
        "fetch tool": get_platform_executable("fetch")
    }
    missing_deps = [name for name, path in dependencies.items() if not Path(path).exists()]

    if missing_deps:
        for name in missing_deps:
            print(f"[!] Error: Dependency '{name}' is missing.")
        print("Please run 'install.bat' first to download all required files.")
        sys.exit(1)

    print("[+] All dependencies are present.\n")

def extract_image_avb_info(image_path):
    """Extracts AVB metadata from an image file using regex for robustness."""
    info_proc = run_command(
        [str(PYTHON_EXE), str(AVBTOOL_PY), "info_image", "--image", str(image_path)],
        capture=True
    )
    
    output = info_proc.stdout.strip()
    info = {}
    props_args = []

    partition_size_match = re.search(r"^Image size:\s*(\d+)\s*bytes", output, re.MULTILINE)
    if partition_size_match:
        info['partition_size'] = partition_size_match.group(1)
    
    data_size_match = re.search(r"Original image size:\s*(\d+)\s*bytes", output)
    if data_size_match:
        info['data_size'] = data_size_match.group(1)
    else:
        desc_size_match = re.search(r"^\s*Image Size:\s*(\d+)\s*bytes", output, re.MULTILINE)
        if desc_size_match:
            info['data_size'] = desc_size_match.group(1)

    patterns = {
        'name': r"Partition Name:\s*(\S+)",
        'salt': r"Salt:\s*([0-9a-fA-F]+)",
        'algorithm': r"Algorithm:\s*(\S+)",
        'pubkey_sha1': r"Public key \(sha1\):\s*([0-9a-fA-F]+)",
    }
    
    header_section = output.split('Descriptors:')[0]
    rollback_match = re.search(r"Rollback Index:\s*(\d+)", header_section)
    if rollback_match:
        info['rollback'] = rollback_match.group(1)
        
    for key, pattern in patterns.items():
        if key not in info:
            match = re.search(pattern, output)
            if match:
                info[key] = match.group(1)

    for line in output.split('\n'):
        if line.strip().startswith("Prop:"):
            parts = line.split('->')
            key = parts[0].split(':')[-1].strip()
            val = parts[1].strip()[1:-1]
            info[key] = val
            props_args.extend(["--prop", f"{key}:{val}"])
            
    info['props_args'] = props_args

    return info


# --- Core Functions ---
def patch_boot_with_root():
    """Patches boot.img with KernelSU and returns the path to the patched image."""
    print("--- Starting boot.img patching process ---")
    magiskboot_exe = get_platform_executable("magiskboot")
    fetch_exe = get_platform_executable("fetch")
    
    patched_boot_path = BASE_DIR / "boot.root.img"

    if not magiskboot_exe.exists():
        print(f"[!] '{magiskboot_exe.name}' not found. Attempting to download...")
        if platform.system() == "Windows":
            url = 'https://github.com/CYRUS-STUDIO/MagiskBootWindows/raw/main/magiskboot.exe'
            try:
                response = requests.get(url, stream=True, timeout=30)
                response.raise_for_status()
                with open(magiskboot_exe, 'wb') as f:
                    shutil.copyfileobj(response.raw, f)
                print("[+] Download successful.")
            except requests.RequestException as e:
                print(f"Error downloading magiskboot: {e}", file=sys.stderr)
                sys.exit(1)
        else:
            print(f"[!] Auto-download for {platform.system()} is not supported. Please add it to the 'tools' folder manually.")
            sys.exit(1)

    if not fetch_exe.exists():
         print(f"[!] '{fetch_exe.name}' not found. Please run install.bat")
         sys.exit(1)

    if platform.system() != "Windows":
        os.chmod(magiskboot_exe, 0o755)
        os.chmod(fetch_exe, 0o755)

    boot_img = BASE_DIR / "boot.img"
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
        run_command([str(magiskboot_exe), "unpack", "boot.img"])
        if not (WORK_DIR / "kernel").exists():
            print("[!] Failed to unpack boot.img. The image might be invalid.")
            sys.exit(1)
        print("[+] Unpack successful.")

        print("\n[2/8] Verifying kernel version...")
        result = run_command([str(PYTHON_EXE), str(GET_KERNEL_VER_PY), "kernel"])
        target_kernel_version = result.stdout.strip()

        if not re.match(r"\d+\.\d+\.\d+", target_kernel_version):
             print(f"[!] Invalid kernel version returned from script: '{target_kernel_version}'")
             sys.exit(1)
        
        print(f"[+] Target kernel version for download: {target_kernel_version}")

        print("\n[3/8] Downloading GKI Kernel with fetch...")
        asset_pattern = f".*{target_kernel_version}.*AnyKernel3.zip"
        fetch_command = [
            str(fetch_exe), "--repo", REPO_URL, "--tag", RELEASE_TAG,
            "--release-asset", asset_pattern, "."
        ]
        run_command(fetch_command)

        downloaded_files = list(Path(".").glob(f"*{target_kernel_version}*AnyKernel3.zip"))
        if not downloaded_files:
            print(f"[!] Failed to download AnyKernel3.zip for kernel {target_kernel_version}.")
            sys.exit(1)
        shutil.move(downloaded_files[0], ANYKERNEL_ZIP_FILENAME)
        print("[+] Download complete.")

        print("\n[4/8] Extracting new kernel image...")
        extracted_kernel_dir = WORK_DIR / "extracted_kernel"
        with zipfile.ZipFile(ANYKERNEL_ZIP_FILENAME, 'r') as zip_ref:
            zip_ref.extractall(extracted_kernel_dir)
        if not (extracted_kernel_dir / "Image").exists():
            print("[!] 'Image' file not found in the downloaded zip.")
            sys.exit(1)
        print("[+] Extraction successful.")

        print("\n[5/8] Replacing original kernel with the new one...")
        shutil.move(str(extracted_kernel_dir / "Image"), "kernel")
        print("[+] Kernel replaced.")

        print("\n[6/8] Repacking boot image...")
        run_command([str(magiskboot_exe), "repack", "boot.img"])
        if not (WORK_DIR / "new-boot.img").exists():
            print("[!] Failed to repack the boot image.")
            sys.exit(1)
        shutil.move("new-boot.img", patched_boot_path)
        print("[+] Repack successful.")

        print("\n[7/8] Downloading KernelSU Manager APKs...")
        if list(BASE_DIR.glob("KernelSU*.apk")):
            print("[+] KernelSU Manager APK already exists. Skipping download.")
        else:
            ksu_apk_command = [
                str(fetch_exe), "--repo", f"https://github.com/{KSU_APK_REPO}", "--tag", KSU_APK_TAG,
                "--release-asset", ".*\\.apk", str(BASE_DIR)
            ]
            run_command(ksu_apk_command)
            print("[+] KernelSU Manager APKs downloaded to the main directory (if found).")

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
    """Converts vendor_boot and remakes vbmeta, without rooting."""
    check_dependencies()

    print("[*] Cleaning up old folders...")
    if OUTPUT_DIR.exists():
        shutil.rmtree(OUTPUT_DIR)
    print()

    print("--- Backing up original images ---")
    vendor_boot_img = BASE_DIR / "vendor_boot.img"
    vbmeta_img = BASE_DIR / "vbmeta.img"
    required_images = {"vendor_boot.img": vendor_boot_img, "vbmeta.img": vbmeta_img}

    for name, path in required_images.items():
        if not path.exists():
            print(f"[!] '{name}' not found! Aborting.")
            sys.exit(1)

    vendor_boot_bak = BASE_DIR / "vendor_boot.bak.img"
    vbmeta_bak = BASE_DIR / "vbmeta.bak.img"
    shutil.move(vendor_boot_img, vendor_boot_bak)
    shutil.copy(vbmeta_img, vbmeta_bak)
    print("[+] Backup complete.\n")

    print("--- Starting PRC/ROW Conversion ---")
    run_command([str(PYTHON_EXE), str(EDIT_IMAGES_PY), "vndrboot", str(vendor_boot_bak)])

    vendor_boot_prc = BASE_DIR / "vendor_boot_prc.img"
    print("\n[*] Verifying conversion result...")
    if not vendor_boot_prc.exists():
        print("[!] 'vendor_boot_prc.img' was not created. No changes made.")
        sys.exit(1)
    print("[+] Conversion to PRC successful.\n")

    print("--- Extracting image information ---")
    vbmeta_info = extract_image_avb_info(vbmeta_bak)
    vendor_boot_info = extract_image_avb_info(vendor_boot_bak)
    print("[+] Information extracted.\n")

    prop_key = "com.android.build.vendor_boot.fingerprint"
    prop_val = vendor_boot_info.get(prop_key)

    print("--- Adding Hash Footer to vendor_boot ---")
    
    for key in ['partition_size', 'name', 'rollback', 'salt']:
        if key not in vendor_boot_info:
            raise KeyError(f"Could not find '{key}' in '{vendor_boot_bak.name}' AVB info.")

    add_hash_footer_cmd = [
        str(PYTHON_EXE), str(AVBTOOL_PY), "add_hash_footer",
        "--image", str(vendor_boot_prc),
        "--partition_size", vendor_boot_info['partition_size'],
        "--partition_name", vendor_boot_info['name'],
        "--rollback_index", vendor_boot_info['rollback'],
        "--salt", vendor_boot_info['salt']
    ]
    if prop_val:
        with open("prop_val.tmp", "w", encoding='utf-8') as f:
            f.write(prop_val)
        add_hash_footer_cmd.extend(["--prop_from_file", f"{prop_key}:prop_val.tmp"])

    run_command(add_hash_footer_cmd)
    Path("prop_val.tmp").unlink(missing_ok=True)
    
    key_map = {
        "2597c218aae470a130f61162feaae70afd97f011": AVB_DIR / "testkey_rsa4096.pem",
        "cdbb77177f731920bbe0a0f94f84d9038ae0617d": AVB_DIR / "testkey_rsa2048.pem"
    }
    
    vbmeta_pubkey = vbmeta_info.get('pubkey_sha1')
    key_file = key_map.get(vbmeta_pubkey)

    print(f"--- Remaking vbmeta.img ---")
    print("[*] Verifying vbmeta key...")
    if not key_file:
        print(f"[!] Public key SHA1 '{vbmeta_pubkey}' from vbmeta did not match known keys. Aborting.")
        sys.exit(1)
    print(f"[+] Matched {key_file.name}.\n")

    print("[*] Remaking 'vbmeta.img' using descriptors from backup...")
    remake_cmd = [
        str(PYTHON_EXE), str(AVBTOOL_PY), "make_vbmeta_image",
        "--output", str(vbmeta_img),
        "--key", str(key_file),
        "--algorithm", vbmeta_info['algorithm'],
        "--padding_size", "8192",
        "--include_descriptors_from_image", str(vbmeta_bak),
        "--include_descriptors_from_image", str(vendor_boot_prc)
    ]
        
    run_command(remake_cmd)
    print()

    finalize_images()

def finalize_images():
    """Finalizes the process by moving images to their final destinations."""
    print("--- Finalizing ---")
    print("[*] Renaming final images...")
    final_vendor_boot = BASE_DIR / "vendor_boot.img"
    shutil.move(BASE_DIR / "vendor_boot_prc.img", final_vendor_boot)

    final_images = [final_vendor_boot, BASE_DIR / "vbmeta.img"]

    print("\n[*] Moving final images to 'output' folder...")
    OUTPUT_DIR.mkdir(exist_ok=True)
    for img in final_images:
        shutil.move(img, OUTPUT_DIR / img.name)

    print("\n[*] Moving backup files to 'backup' folder...")
    BACKUP_DIR.mkdir(exist_ok=True)
    for bak_file in BASE_DIR.glob("*.bak.img"):
        shutil.move(bak_file, BACKUP_DIR / bak_file.name)
    print()

    print("=" * 61)
    print("  SUCCESS!")
    print("  Final images have been saved to the 'output' folder.")
    print("=" * 61)
    
def root_boot_only():
    """Patches boot.img with KernelSU and places it in the output_root folder."""
    print(f"[*] Cleaning up old '{OUTPUT_ROOT_DIR.name}' folder...")
    if OUTPUT_ROOT_DIR.exists():
        shutil.rmtree(OUTPUT_ROOT_DIR)
    OUTPUT_ROOT_DIR.mkdir(exist_ok=True)
    print()
    
    check_dependencies()

    patched_boot_path = patch_boot_with_root()

    if patched_boot_path and patched_boot_path.exists():
        print("\n--- Finalizing ---")
        final_boot_img = OUTPUT_ROOT_DIR / "boot.img"
        
        print("\n[*] Verifying boot image key...")
        key_map = {
            "2597c218aae470a130f61162feaae70afd97f011": AVB_DIR / "testkey_rsa4096.pem",
            "cdbb77177f731920bbe0a0f94f84d9038ae0617d": AVB_DIR / "testkey_rsa2048.pem"
        }
        process_boot_image(key_map, patched_boot_path)

        print(f"\n[*] Moving final image to '{OUTPUT_ROOT_DIR.name}' folder...")
        shutil.move(patched_boot_path, final_boot_img)

        print("\n[*] Moving backup file to 'backup' folder...")
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


def process_boot_image(key_map, image_to_process):
    """Adds a hash footer to a boot image."""
    boot_bak_img = BASE_DIR / "boot.bak.img"
    boot_info = extract_image_avb_info(boot_bak_img)
    
    for key in ['partition_size', 'name', 'rollback', 'salt', 'algorithm', 'pubkey_sha1']:
        if key not in boot_info:
            raise KeyError(f"Could not find '{key}' in '{boot_bak_img.name}' AVB info.")
            
    boot_pubkey = boot_info.get('pubkey_sha1')
    key_file = key_map.get(boot_pubkey)
    
    if not key_file:
        print(f"[!] Public key SHA1 '{boot_pubkey}' from boot.img did not match known keys. Cannot add footer.")
        sys.exit(1)

    print(f"[+] Matched {key_file.name}.")
    print(f"\n[*] Adding new hash footer to '{image_to_process.name}'...")
    add_footer_cmd = [
        str(PYTHON_EXE), str(AVBTOOL_PY), "add_hash_footer",
        "--image", str(image_to_process), 
        "--key", str(key_file),
        "--algorithm", boot_info['algorithm'], 
        "--partition_size", boot_info['partition_size'],
        "--partition_name", boot_info['name'], 
        "--rollback_index", boot_info['rollback'],
        "--salt", boot_info['salt'], 
        *boot_info.get('props_args', [])
    ]
    run_command(add_footer_cmd)


def edit_devinfo_persist():
    """Backs up and modifies devinfo.img and persist.img."""
    print("--- Starting devinfo & persist patching process ---")
    
    devinfo_img = BASE_DIR / "devinfo.img"
    persist_img = BASE_DIR / "persist.img"
    
    if not devinfo_img.exists() or not persist_img.exists():
        print("[!] Error: 'devinfo.img' and/or 'persist.img' not found. Please place them in the main directory.")
        sys.exit(1)
        
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    backup_critical_dir = BASE_DIR / f"backup_critical_{timestamp}"
    backup_critical_dir.mkdir(exist_ok=True)
    
    print(f"[*] Backing up critical images to '{backup_critical_dir.name}'...")
    shutil.copy(devinfo_img, backup_critical_dir)
    shutil.copy(persist_img, backup_critical_dir)
    print("[+] Backup complete.\n")

    print(f"[*] Cleaning up old '{OUTPUT_DP_DIR.name}' folder...")
    if OUTPUT_DP_DIR.exists():
        shutil.rmtree(OUTPUT_DP_DIR)
    OUTPUT_DP_DIR.mkdir(exist_ok=True)

    print("[*] Running patch script...")
    run_command([str(PYTHON_EXE), str(EDIT_IMAGES_PY), "dp"])

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


def show_image_info(files):
    """Displays information about image files, searching directories for .img files."""
    all_files = []
    for f in files:
        path = Path(f)
        if path.is_dir():
            all_files.extend(path.rglob('*.img'))
        elif path.is_file():
            all_files.append(path)

    if not all_files:
        print("No .img files found in the provided paths.")
        return
        
    output_lines = [
        "\n" + "=" * 42,
        "  Sorted and Processing Images...",
        "=" * 42 + "\n"
    ]
    print("\n".join(output_lines))

    for file_path in sorted(all_files):
        info_header = f"Processing file: {file_path.name}\n---------------------------------"
        print(info_header)
        output_lines.append(info_header)

        if not file_path.exists():
            not_found_msg = f"File not found: {file_path}"
            print(not_found_msg)
            output_lines.append(not_found_msg)
            continue

        try:
            process = run_command(
                [str(PYTHON_EXE), str(AVBTOOL_PY), "info_image", "--image", str(file_path)],
            )
            output_lines.append(process.stdout.strip())
        except subprocess.CalledProcessError as e:
            error_message = f"Failed to get info from {file_path.name}"
            print(error_message, file=sys.stderr)
            if e.stderr:
                print(e.stderr.strip(), file=sys.stderr)
            output_lines.append(error_message)
        finally:
            output_lines.append("---------------------------------\n")

    try:
        timestamp = datetime.now().strftime("%Y-%m-%d_%H%M%S")
        output_filename = BASE_DIR / f"image_info_{timestamp}.txt"
        with open(output_filename, "w", encoding="utf-8") as f:
            f.write("\n".join(output_lines))
        print(f"[*] Image info saved to: {output_filename}")
    except IOError as e:
        print(f"[!] Error saving info to file: {e}", file=sys.stderr)

def main():
    """Main function to parse arguments and call appropriate functions."""
    parser = argparse.ArgumentParser(description="Android Image Patcher and AVB Tool.")
    subparsers = parser.add_subparsers(dest="command", required=True, help="Available commands")

    subparsers.add_parser("convert", help="Convert vendor_boot region and remake vbmeta.")
    subparsers.add_parser("root", help="Patch boot.img with KernelSU.")
    subparsers.add_parser("edit_dp", help="Edit devinfo and persist images.")
    parser_info = subparsers.add_parser("info", help="Display AVB info for image files or directories.")
    parser_info.add_argument("files", nargs='+', help="Image file(s) or folder(s) to inspect.")

    args = parser.parse_args()

    try:
        if args.command == "convert":
            convert_images()
        elif args.command == "root":
            root_boot_only()
        elif args.command == "edit_dp":
            edit_devinfo_persist()
        elif args.command == "info":
            show_image_info(args.files)
    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, SystemExit) as e:
        if not isinstance(e, SystemExit):
            print(f"\nAn unexpected error occurred: {e}", file=sys.stderr)
    except KeyError as e:
        print(f"\nAn error occurred while processing image info: {e}", file=sys.stderr)
        print("Please check if the image is valid and contains the necessary AVB metadata.", file=sys.stderr)

    finally:
        print()
        if platform.system() == "Windows":
            os.system("pause")

if __name__ == "__main__":
    main()