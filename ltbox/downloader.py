import os
import platform
import shutil
import subprocess
import sys
import zipfile
import tarfile
import re
import json
from pathlib import Path
from typing import Dict, List, Optional

sys.path.insert(0, str(Path(__file__).parent.parent.resolve()))

from ltbox import constants as const
from ltbox import utils
from ltbox.i18n import get_string, load_lang as i18n_load_lang

class ToolError(Exception):
    pass

def download_resource(url: str, dest_path: Path) -> None:
    import requests
    msg = get_string("dl_downloading").format(filename=dest_path.name)
    print(msg)
    try:
        with requests.get(url, stream=True, allow_redirects=True) as response:
            response.raise_for_status()
            with open(dest_path, 'wb') as f:
                for chunk in response.iter_content(chunk_size=8192):
                    f.write(chunk)
        msg_success = get_string("dl_download_success").format(filename=dest_path.name)
        print(msg_success)
    except Exception as e:
        msg_err = get_string("dl_download_failed").format(url=url, error=e)
        print(msg_err, file=sys.stderr)
        if dest_path.exists():
            dest_path.unlink()
        raise ToolError(f"Download failed for {dest_path.name}")

def extract_archive_files(archive_path: Path, extract_map: Dict[str, Path]) -> None:
    msg = get_string("dl_extracting").format(filename=archive_path.name)
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
                            print(get_string("dl_extracted_file").format(filename=target_path.name))
        else:
            with zipfile.ZipFile(archive_path, 'r') as zf:
                for member in zf.infolist():
                    if member.filename in extract_map:
                        target_path = extract_map[member.name]
                        with zf.open(member) as source, open(target_path, "wb") as target:
                            shutil.copyfileobj(source, target)
                        print(get_string("dl_extracted_file").format(filename=target_path.name))
                        
    except Exception as e:
        msg_err = get_string("dl_extract_failed").format(filename=archive_path.name, error=e)
        print(msg_err, file=sys.stderr)
        raise ToolError(f"Extraction failed for {archive_path.name}")

def _run_fetch_command(args: List[str]) -> subprocess.CompletedProcess:
    fetch_exe = const.DOWNLOAD_DIR / "fetch.exe"
    if not fetch_exe.exists():
        print(get_string("dl_fetch_not_found"))
        raise FileNotFoundError("fetch.exe not found")
    
    command = [str(fetch_exe)] + args
    return utils.run_command(command, capture=True)

def _ensure_tool_from_github_release(
    tool_name: str, 
    exe_name_in_zip: str, 
    repo_url: str, 
    tag: str, 
    asset_patterns: Dict[str, str]
) -> Path:
    tool_exe = const.DOWNLOAD_DIR / f"{tool_name}.exe"
    if tool_exe.exists():
        return tool_exe

    print(get_string("dl_tool_not_found").format(tool_name=tool_exe.name))
    const.DOWNLOAD_DIR.mkdir(exist_ok=True)
    
    arch = platform.machine()
    asset_pattern = asset_patterns.get(arch)
    if not asset_pattern:
        msg = get_string("dl_unsupported_arch").format(arch=arch, tool_name=tool_name)
        print(msg, file=sys.stderr)
        raise ToolError(f"Unsupported architecture for {tool_name}")

    msg = get_string("dl_detect_arch").format(arch=arch, pattern=asset_pattern)
    print(msg)

    try:
        fetch_command = [
            "--repo", repo_url,
            "--tag", tag,
            "--release-asset", asset_pattern,
            str(const.DOWNLOAD_DIR)
        ]
        _run_fetch_command(fetch_command)

        downloaded_zips = list(const.DOWNLOAD_DIR.glob(f"*{tool_name}*.zip"))
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

            zip_ref.extract(exe_info, path=const.DOWNLOAD_DIR)
            extracted_path = const.DOWNLOAD_DIR / exe_info.filename
            
            if extracted_path != tool_exe:
                shutil.move(extracted_path, tool_exe)
            
            parent_dir = extracted_path.parent
            if parent_dir.is_dir() and parent_dir != const.DOWNLOAD_DIR:
                 try:
                    parent_dir.rmdir()
                 except OSError:
                    shutil.rmtree(parent_dir, ignore_errors=True)

        downloaded_zip_path.unlink()
        print(get_string("dl_tool_success").format(tool_name=tool_name))
        return tool_exe

    except Exception as e:
        msg_err = get_string("dl_tool_failed").format(tool_name=tool_name, error=e)
        print(msg_err, file=sys.stderr)
        raise ToolError(f"Failed to ensure {tool_name}")

def ensure_fetch() -> Path:
    tool_exe = const.DOWNLOAD_DIR / "fetch.exe"
    if tool_exe.exists():
        return tool_exe
    
    const.DOWNLOAD_DIR.mkdir(exist_ok=True)
    
    asset_patterns = {
        'AMD64': "fetch_windows_amd64.exe",
        'ARM64': "fetch_windows_amd64.exe",
        'I386': "fetch_windows_386.exe"
    }
    arch = platform.machine()
    asset_name = asset_patterns.get(arch)
    if not asset_name:
         raise ToolError(f"Unsupported architecture for fetch: {arch}")

    url = f"{const.FETCH_REPO_URL}/releases/download/{const.FETCH_VERSION}/{asset_name}"
    download_resource(url, tool_exe)
    return tool_exe

def ensure_platform_tools() -> None:
    if const.ADB_EXE.exists() and const.FASTBOOT_EXE.exists():
        return
    
    print(get_string("dl_platform_not_found"))
    const.DOWNLOAD_DIR.mkdir(exist_ok=True)
    temp_zip_path = const.DOWNLOAD_DIR / "platform-tools.zip"
    
    download_resource(const.PLATFORM_TOOLS_ZIP_URL, temp_zip_path)
    
    try:
        with zipfile.ZipFile(temp_zip_path) as zf:
            for member in zf.infolist():
                if member.is_dir():
                    continue
                
                if re.match(r"^platform-tools/[^/]+$", member.filename):
                    file_name = Path(member.filename).name
                    target_path = const.DOWNLOAD_DIR / file_name
                    with zf.open(member) as source, open(target_path, "wb") as target:
                        shutil.copyfileobj(source, target)
                        
        temp_zip_path.unlink()
        print(get_string("dl_platform_success"))
        
    except Exception as e:
        msg_err = get_string("dl_platform_failed").format(error=e)
        print(msg_err, file=sys.stderr)
        if temp_zip_path.exists():
            temp_zip_path.unlink()
        raise ToolError("Failed to process platform-tools")

def ensure_avb_tools() -> None:
    key1 = const.DOWNLOAD_DIR / "testkey_rsa4096.pem"
    key2 = const.DOWNLOAD_DIR / "testkey_rsa2048.pem"
    
    if const.AVBTOOL_PY.exists() and key1.exists() and key2.exists():
        return

    print(get_string("dl_avb_not_found"))
    const.DOWNLOAD_DIR.mkdir(exist_ok=True)
    temp_tar_path = const.DOWNLOAD_DIR / "avb.tar.gz"
    
    download_resource(const.AVB_ARCHIVE_URL, temp_tar_path)

    files_to_extract = {
        "avbtool.py": const.AVBTOOL_PY,
        "test/data/testkey_rsa4096.pem": key1,
        "test/data/testkey_rsa2048.pem": key2,
    }

    extract_archive_files(temp_tar_path, files_to_extract)
    temp_tar_path.unlink()
    print(get_string("dl_avb_ready"))

def ensure_magiskboot() -> Path:
    asset_patterns = {
        'AMD64': "magiskboot-.*-windows-.*-x86_64-standalone\\.zip",
        'ARM64': "magiskboot-.*-windows-.*-arm64-standalone\\.zip",
    }
    
    try:
        return _ensure_tool_from_github_release(
            tool_name="magiskboot",
            exe_name_in_zip="magiskboot.exe",
            repo_url=const.MAGISKBOOT_REPO_URL,
            tag=const.MAGISKBOOT_TAG,
            asset_patterns=asset_patterns
        )
    except ToolError:
        sys.exit(1)

def get_gki_kernel(kernel_version: str, work_dir: Path) -> Path:
    print(get_string("dl_gki_downloading"))
    asset_pattern = f".*{kernel_version}.*Normal-AnyKernel3.zip"
    fetch_command = [
        "--repo", const.REPO_URL, "--tag", const.RELEASE_TAG,
        "--release-asset", asset_pattern, str(work_dir)
    ]
    _run_fetch_command(fetch_command)

    downloaded_files = list(work_dir.glob(f"*{kernel_version}*Normal-AnyKernel3.zip"))
    if not downloaded_files:
        print(get_string("dl_gki_download_fail").format(version=kernel_version))
        sys.exit(1)
    
    anykernel_zip = work_dir / const.ANYKERNEL_ZIP_FILENAME
    shutil.move(downloaded_files[0], anykernel_zip)
    print(get_string("dl_gki_download_ok"))

    print(get_string("dl_gki_extracting"))
    extracted_kernel_dir = work_dir / "extracted_kernel"
    with zipfile.ZipFile(anykernel_zip, 'r') as zip_ref:
        zip_ref.extractall(extracted_kernel_dir)
    
    kernel_image = extracted_kernel_dir / "Image"
    if not kernel_image.exists():
        print(get_string("dl_gki_image_missing"))
        sys.exit(1)
    print(get_string("dl_gki_extract_ok"))
    return kernel_image

def download_ksu_apk(target_dir: Path) -> None:
    print(get_string("dl_ksu_downloading"))
    if list(target_dir.glob("*spoofed*.apk")):
        print(get_string("dl_ksu_exists"))
    else:
        ksu_apk_command = [
            "--repo", f"https://github.com/{const.KSU_APK_REPO}", "--tag", const.KSU_APK_TAG,
            "--release-asset", ".*spoofed.*\\.apk", str(target_dir)
        ]
        _run_fetch_command(ksu_apk_command)
        print(get_string("dl_ksu_success"))

if __name__ == "__main__":
    lang_code = "en" 
    if "--lang" in sys.argv:
        try:
            lang_code = sys.argv[sys.argv.index("--lang") + 1]
        except (IndexError, ValueError):
            pass 
    
    i18n_load_lang(lang_code) 
    
    if len(sys.argv) > 1 and "install_base_tools" in sys.argv:
        print(get_string("dl_base_installing"))
        const.DOWNLOAD_DIR.mkdir(exist_ok=True)
        try:
            print(get_string("utils_check_deps"))
            req_path = const.BASE_DIR / "requirements.txt"
            subprocess.run(
                [str(const.PYTHON_EXE), "-m", "pip", "install", "-r", str(req_path)],
                check=True
            )
            
            ensure_fetch()
            ensure_platform_tools()
            ensure_avb_tools()
            print(get_string("dl_base_complete"))
        except Exception as e:
            msg = get_string("dl_base_error").format(error=e)
            print(msg, file=sys.stderr)
            if platform.system() == "Windows":
                os.system("pause")
            sys.exit(1)