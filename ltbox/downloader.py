import platform
import shutil
import subprocess
import sys
import zipfile
import tarfile
import requests
import re
import json
from pathlib import Path
from typing import Dict, List, Optional

sys.path.insert(0, str(Path(__file__).parent.parent.resolve()))

from ltbox.constants import *
from ltbox import utils

APP_DIR = Path(__file__).parent.resolve()
LANG_DIR = APP_DIR / "lang"

class ToolError(Exception):
    pass

def load_language(lang_code: str) -> Dict[str, str]:
    if not lang_code:
        return {}
    lang_file_path = LANG_DIR / f"{lang_code}.json"
    if not lang_file_path.exists():
        print(f"[!] Warning: Language file {lang_file_path.name} not found, falling back to defaults.")
        return {}
    try:
        with open(lang_file_path, 'r', encoding='utf-8') as f:
            return json.load(f)
    except Exception as e:
        print(f"[!] Warning: Failed to load language file {lang_file_path.name}: {e}", file=sys.stderr)
        return {}

def download_resource(url: str, dest_path: Path, lang: Optional[Dict[str, str]] = None) -> None:
    lang = lang or {}
    msg = lang.get('dl_downloading', "[*] Downloading {filename}...").format(filename=dest_path.name)
    print(msg)
    try:
        with requests.get(url, stream=True, allow_redirects=True) as response:
            response.raise_for_status()
            with open(dest_path, 'wb') as f:
                for chunk in response.iter_content(chunk_size=8192):
                    f.write(chunk)
        msg_success = lang.get('dl_download_success', "[+] Downloaded {filename} successfully.").format(filename=dest_path.name)
        print(msg_success)
    except Exception as e:
        msg_err = lang.get('dl_download_failed', "[!] Failed to download {url}: {error}").format(url=url, error=e)
        print(msg_err, file=sys.stderr)
        if dest_path.exists():
            dest_path.unlink()
        raise ToolError(f"Download failed for {dest_path.name}")

def extract_archive_files(archive_path: Path, extract_map: Dict[str, Path], lang: Optional[Dict[str, str]] = None) -> None:
    lang = lang or {}
    msg = lang.get('dl_extracting', "[*] Extracting files from {filename}...").format(filename=archive_path.name)
    print(msg)
    try:
        is_tar = archive_path.suffix == '.gz' or archive_path.suffix == '.tar'
        
        if is_tar:
            with tarfile.open(archive_path, "r:*") as tf:
                for member in tf:
                    if member.name in extract_map:
                        target_path = extract_map[member.name]
                        f = tf.extractfile(member)
                        if f:
                            with open(target_path, "wb") as target:
                                shutil.copyfileobj(f, target)
                            print(lang.get('dl_extracted_file', "  > Extracted {filename}").format(filename=target_path.name))
        else:
            with zipfile.ZipFile(archive_path, 'r') as zf:
                for member in zf.infolist():
                    if member.filename in extract_map:
                        target_path = extract_map[member.filename]
                        with zf.open(member) as source, open(target_path, "wb") as target:
                            shutil.copyfileobj(source, target)
                        print(lang.get('dl_extracted_file', "  > Extracted {filename}").format(filename=target_path.name))
                        
    except Exception as e:
        msg_err = lang.get('dl_extract_failed', "[!] Failed to extract archive {filename}: {error}").format(filename=archive_path.name, error=e)
        print(msg_err, file=sys.stderr)
        raise ToolError(f"Extraction failed for {archive_path.name}")

def _run_fetch_command(args: List[str], lang: Optional[Dict[str, str]] = None) -> subprocess.CompletedProcess:
    lang = lang or {}
    fetch_exe = DOWNLOAD_DIR / "fetch.exe"
    if not fetch_exe.exists():
        print(lang.get('dl_fetch_not_found', "[!] 'fetch.exe' not found. Cannot proceed."))
        raise FileNotFoundError("fetch.exe not found")
    
    command = [str(fetch_exe)] + args
    return utils.run_command(command, capture=True)

def _ensure_tool_from_github_release(
    tool_name: str, 
    exe_name_in_zip: str, 
    repo_url: str, 
    tag: str, 
    asset_patterns: Dict[str, str],
    lang: Optional[Dict[str, str]] = None
) -> Path:
    lang = lang or {}
    tool_exe = DOWNLOAD_DIR / f"{tool_name}.exe"
    if tool_exe.exists():
        return tool_exe

    print(lang.get('dl_tool_not_found', "[!] '{tool_name}' not found. Attempting to download...").format(tool_name=tool_exe.name))
    DOWNLOAD_DIR.mkdir(exist_ok=True)
    
    arch = platform.machine()
    asset_pattern = asset_patterns.get(arch)
    if not asset_pattern:
        msg = lang.get('dl_unsupported_arch', "[!] Unsupported architecture: {arch} for {tool_name}. Aborting.").format(arch=arch, tool_name=tool_name)
        print(msg, file=sys.stderr)
        raise ToolError(f"Unsupported architecture for {tool_name}")

    msg = lang.get('dl_detect_arch', "[*] Detected {arch} architecture. Downloading asset matching '{pattern}'...").format(arch=arch, pattern=asset_pattern)
    print(msg)

    try:
        fetch_command = [
            "--repo", repo_url,
            "--tag", tag,
            "--release-asset", asset_pattern,
            str(DOWNLOAD_DIR)
        ]
        _run_fetch_command(fetch_command, lang)

        downloaded_zips = list(DOWNLOAD_DIR.glob(f"*{tool_name}*.zip"))
        if not downloaded_zips:
            raise FileNotFoundError(f"Failed to find downloaded zip for {tool_name}")

        downloaded_zip_path = downloaded_zips[0]

        with zipfile.ZipFile(downloaded_zip_path, 'r') as zip_ref:
            exe_info = None
            for member in zip_ref.infolist():
                if member.filename.endswith(exe_name_in_zip):
                    exe_info = member
                    break
            
            if not exe_info:
                raise FileNotFoundError(f"'{exe_name_in_zip}' not found inside {downloaded_zip_path.name}")

            zip_ref.extract(exe_info, path=DOWNLOAD_DIR)
            extracted_path = DOWNLOAD_DIR / exe_info.filename
            
            if extracted_path != tool_exe:
                shutil.move(extracted_path, tool_exe)
            
            parent_dir = extracted_path.parent
            if parent_dir.is_dir() and parent_dir != DOWNLOAD_DIR:
                 try:
                    parent_dir.rmdir()
                 except OSError:
                    shutil.rmtree(parent_dir, ignore_errors=True)

        downloaded_zip_path.unlink()
        print(lang.get('dl_tool_success', "[+] {tool_name} download and extraction successful.").format(tool_name=tool_name))
        return tool_exe

    except Exception as e:
        msg_err = lang.get('dl_tool_failed', "[!] Error downloading or extracting {tool_name}: {error}").format(tool_name=tool_name, error=e)
        print(msg_err, file=sys.stderr)
        raise ToolError(f"Failed to ensure {tool_name}")

def ensure_fetch(lang: Optional[Dict[str, str]] = None) -> Path:
    tool_exe = DOWNLOAD_DIR / "fetch.exe"
    if tool_exe.exists():
        return tool_exe
    
    DOWNLOAD_DIR.mkdir(exist_ok=True)
    
    asset_patterns = {
        'AMD64': "fetch_windows_amd64.exe",
        'ARM64': "fetch_windows_amd64.exe",
        'I386': "fetch_windows_386.exe"
    }
    arch = platform.machine()
    asset_name = asset_patterns.get(arch)
    if not asset_name:
         raise ToolError(f"Unsupported architecture for fetch: {arch}")

    url = f"{FETCH_REPO_URL}/releases/download/{FETCH_VERSION}/{asset_name}"
    download_resource(url, tool_exe, lang)
    return tool_exe

def ensure_platform_tools(lang: Optional[Dict[str, str]] = None) -> None:
    lang = lang or {}
    if ADB_EXE.exists() and FASTBOOT_EXE.exists():
        return
    
    print(lang.get('dl_platform_not_found', "[!] platform-tools (adb, fastboot) not found. Downloading..."))
    DOWNLOAD_DIR.mkdir(exist_ok=True)
    temp_zip_path = DOWNLOAD_DIR / "platform-tools.zip"
    
    download_resource(PLATFORM_TOOLS_ZIP_URL, temp_zip_path, lang)
    
    try:
        with zipfile.ZipFile(temp_zip_path) as zf:
            for member in zf.infolist():
                if member.is_dir():
                    continue
                
                if re.match(r"^platform-tools/[^/]+$", member.filename):
                    file_name = Path(member.filename).name
                    target_path = DOWNLOAD_DIR / file_name
                    with zf.open(member) as source, open(target_path, "wb") as target:
                        shutil.copyfileobj(source, target)
                        
        temp_zip_path.unlink()
        print(lang.get('dl_platform_success', "[+] platform-tools extracted successfully."))
        
    except Exception as e:
        msg_err = lang.get('dl_platform_failed', "[!] Failed to extract platform-tools: {error}").format(error=e)
        print(msg_err, file=sys.stderr)
        if temp_zip_path.exists():
            temp_zip_path.unlink()
        raise ToolError("Failed to process platform-tools")

def ensure_avb_tools(lang: Optional[Dict[str, str]] = None) -> None:
    lang = lang or {}
    key1 = DOWNLOAD_DIR / "testkey_rsa4096.pem"
    key2 = DOWNLOAD_DIR / "testkey_rsa2048.pem"
    
    if AVBTOOL_PY.exists() and key1.exists() and key2.exists():
        return

    print(lang.get('dl_avb_not_found', "[!] avbtool or keys not found. Downloading from AOSP..."))
    DOWNLOAD_DIR.mkdir(exist_ok=True)
    temp_tar_path = DOWNLOAD_DIR / "avb.tar.gz"
    
    download_resource(AVB_ARCHIVE_URL, temp_tar_path, lang)

    files_to_extract = {
        "avbtool.py": AVBTOOL_PY,
        "test/data/testkey_rsa4096.pem": key1,
        "test/data/testkey_rsa2048.pem": key2,
    }

    extract_archive_files(temp_tar_path, files_to_extract, lang)
    temp_tar_path.unlink()
    print(lang.get('dl_avb_ready', "[+] avbtool and keys ready."))

def ensure_magiskboot(lang: Optional[Dict[str, str]] = None) -> Path:
    asset_patterns = {
        'AMD64': "magiskboot-.*-windows-.*-x86_64-standalone\\.zip",
        'ARM64': "magiskboot-.*-windows-.*-arm64-standalone\\.zip",
    }
    
    try:
        return _ensure_tool_from_github_release(
            tool_name="magiskboot",
            exe_name_in_zip="magiskboot.exe",
            repo_url=MAGISKBOOT_REPO_URL,
            tag=MAGISKBOOT_TAG,
            asset_patterns=asset_patterns,
            lang=lang
        )
    except ToolError:
        sys.exit(1)

def get_gki_kernel(kernel_version: str, work_dir: Path, lang: Optional[Dict[str, str]] = None) -> Path:
    lang = lang or {}
    print(lang.get('dl_gki_downloading', "\n[3/8] Downloading GKI Kernel with fetch..."))
    asset_pattern = f".*{kernel_version}.*Normal-AnyKernel3.zip"
    fetch_command = [
        "--repo", REPO_URL, "--tag", RELEASE_TAG,
        "--release-asset", asset_pattern, str(work_dir)
    ]
    _run_fetch_command(fetch_command, lang)

    downloaded_files = list(work_dir.glob(f"*{kernel_version}*Normal-AnyKernel3.zip"))
    if not downloaded_files:
        print(lang.get('dl_gki_download_fail', "[!] Failed to download Normal AnyKernel3.zip for kernel {version}.").format(version=kernel_version))
        sys.exit(1)
    
    anykernel_zip = work_dir / ANYKERNEL_ZIP_FILENAME
    shutil.move(downloaded_files[0], anykernel_zip)
    print(lang.get('dl_gki_download_ok', "[+] Download complete."))

    print(lang.get('dl_gki_extracting', "\n[4/8] Extracting new kernel image..."))
    extracted_kernel_dir = work_dir / "extracted_kernel"
    with zipfile.ZipFile(anykernel_zip, 'r') as zip_ref:
        zip_ref.extractall(extracted_kernel_dir)
    
    kernel_image = extracted_kernel_dir / "Image"
    if not kernel_image.exists():
        print(lang.get('dl_gki_image_missing', "[!] 'Image' file not found in the downloaded zip."))
        sys.exit(1)
    print(lang.get('dl_gki_extract_ok', "[+] Extraction successful."))
    return kernel_image

def download_ksu_apk(target_dir: Path, lang: Optional[Dict[str, str]] = None) -> None:
    lang = lang or {}
    print(lang.get('dl_ksu_downloading', "\n[7/8] Downloading KernelSU Next Manager APKs (Spoofed)..."))
    if list(target_dir.glob("*spoofed*.apk")):
        print(lang.get('dl_ksu_exists', "[+] KernelSU Next Manager (Spoofed) APK already exists. Skipping download."))
    else:
        ksu_apk_command = [
            "--repo", f"httpsG://github.com/{KSU_APK_REPO}", "--tag", KSU_APK_TAG,
            "--release-asset", ".*spoofed.*\\.apk", str(target_dir)
        ]
        _run_fetch_command(ksu_apk_command, lang)
        print(lang.get('dl_ksu_success', "[+] KernelSU Next Manager (Spoofed) APKs downloaded to the main directory (if found)."))

if __name__ == "__main__":
    lang_code = "en" 
    if "--lang" in sys.argv:
        try:
            lang_code = sys.argv[sys.argv.index("--lang") + 1]
        except (IndexError, ValueError):
            pass 
    
    lang = load_language(lang_code) 
    
    if len(sys.argv) > 1 and "install_base_tools" in sys.argv:
        print(lang.get('dl_base_installing', "--- Installing Base Tools ---"))
        DOWNLOAD_DIR.mkdir(exist_ok=True)
        try:
            ensure_fetch(lang)
            ensure_platform_tools(lang)
            ensure_avb_tools(lang)
            print(lang.get('dl_base_complete', "--- Base Tools Installation Complete ---"))
        except Exception as e:
            msg = lang.get('dl_base_error', "\n[!] An error occurred during base tool installation: {error}").format(error=e)
            print(msg, file=sys.stderr)
            if platform.system() == "Windows":
                os.system("pause")
            sys.exit(1)