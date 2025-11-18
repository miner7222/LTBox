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
from ltbox.errors import ToolError

def download_resource(url: str, dest_path: Path) -> None:
    import urllib.request
    import shutil
    from urllib.error import URLError, HTTPError

    msg = get_string("dl_downloading").format(filename=dest_path.name)
    print(msg)
    try:
        with urllib.request.urlopen(url) as response, open(dest_path, 'wb') as f:
            if response.status < 200 or response.status >= 300:
                 raise HTTPError(url, response.status, f"HTTP Error {response.status}", response.headers, None)
            shutil.copyfileobj(response, f)

        msg_success = get_string("dl_download_success").format(filename=dest_path.name)
        print(msg_success)
    except (HTTPError, URLError, OSError) as e:
        msg_err = get_string("dl_download_failed").format(url=url, error=e)
        print(msg_err, file=sys.stderr)
        if dest_path.exists():
            dest_path.unlink()
        raise ToolError(get_string("dl_err_download_tool").format(name=dest_path.name))

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
                        
    except (zipfile.BadZipFile, tarfile.TarError, OSError, IOError) as e:
        msg_err = get_string("dl_extract_failed").format(filename=archive_path.name, error=e)
        print(msg_err, file=sys.stderr)
        raise ToolError(get_string("dl_err_extract_tool").format(name=archive_path.name))

def _run_fetch_command(args: List[str]) -> subprocess.CompletedProcess:
    fetch_exe = const.DOWNLOAD_DIR / "fetch.exe"
    if not fetch_exe.exists():
        print(get_string("dl_fetch_not_found"))
        raise FileNotFoundError(get_string("dl_fetch_not_found"))
    
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
        raise ToolError(msg)

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
            raise FileNotFoundError(get_string("dl_err_zip_not_found").format(tool_name=tool_name))

        downloaded_zip_path = downloaded_zips[0]

        with zipfile.ZipFile(downloaded_zip_path, 'r') as zip_ref:
            exe_info = None
            for member in zip_ref.infolist():
                if member.filename.endswith(exe_name_in_zip):
                    exe_info = member
                    break
            
            if not exe_info:
                raise FileNotFoundError(get_string("dl_err_exe_in_zip_not_found").format(exe_name=exe_name_in_zip, zip_name=downloaded_zip_path.name))

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

    except (subprocess.CalledProcessError, FileNotFoundError, zipfile.BadZipFile, OSError, ToolError) as e:
        msg_err = get_string("dl_tool_failed").format(tool_name=tool_name, error=e)
        print(msg_err, file=sys.stderr)
        raise ToolError(msg_err)

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
         raise ToolError(get_string("dl_err_unsupported_arch_fetch").format(arch=arch))

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
        
    except (zipfile.BadZipFile, OSError, IOError) as e:
        msg_err = get_string("dl_platform_failed").format(error=e)
        print(msg_err, file=sys.stderr)
        if temp_zip_path.exists():
            temp_zip_path.unlink()
        raise ToolError(msg_err)

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
    
    return _ensure_tool_from_github_release(
        tool_name="magiskboot",
        exe_name_in_zip="magiskboot.exe",
        repo_url=const.MAGISKBOOT_REPO_URL,
        tag=const.MAGISKBOOT_TAG,
        asset_patterns=asset_patterns
    )

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
        raise ToolError(get_string("dl_gki_download_fail").format(version=kernel_version))
    
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
        raise ToolError(get_string("dl_gki_image_missing"))
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

def get_kernel_version_from_adb(dev: "device.DeviceController") -> str:
    from ltbox import device
    print(get_string("dl_lkm_get_kver"))
    result = utils.run_command(
        [str(const.ADB_EXE), "shell", "cat", "/proc/version"],
        capture=True,
        check=True
    )
    version_string = result.stdout.strip()
    
    match = re.search(r"Linux version (\d+\.\d+)", version_string)
    if not match:
        raise ToolError(get_string("dl_lkm_kver_fail").format(ver=version_string))
    
    kver = match.group(1)
    print(get_string("dl_lkm_kver_found").format(ver=kver))
    return kver

def download_ksuinit(target_path: Path) -> None:
    if target_path.exists():
        target_path.unlink()
    
    url = f"https://github.com/{const.KSU_APK_REPO}/raw/refs/tags/{const.KSU_APK_TAG}/userspace/ksud_magic/bin/aarch64/ksuinit"
    
    import requests
    msg = get_string("dl_downloading").format(filename="ksuinit")
    print(msg)
    try:
        with requests.get(url, stream=True, allow_redirects=True) as response:
            response.raise_for_status()
            with open(target_path, 'wb') as f:
                for chunk in response.iter_content(chunk_size=8192):
                    f.write(chunk)
        msg_success = get_string("dl_download_success").format(filename="ksuinit")
        print(msg_success)
        
        if platform.system() != "Windows":
            os.chmod(target_path, 0o755)
    
    except Exception as e:
        msg_err = get_string("dl_download_failed").format(url=url, error=e)
        print(msg_err, file=sys.stderr)
        if target_path.exists():
            target_path.unlink()
        raise ToolError(get_string("dl_err_download_tool").format(name="ksuinit"))

def get_lkm_kernel(dev: "device.DeviceController", target_path: Path, kernel_version_str: Optional[str] = None) -> None:
    from ltbox import device
    if target_path.exists():
        target_path.unlink()
        
    kernel_version = kernel_version_str
    if not kernel_version:
        kernel_version = get_kernel_version_from_adb(dev)
    else:
        print(get_string("dl_lkm_kver_found").format(ver=kernel_version))
    
    asset_pattern_regex = f"android.*-{kernel_version}_kernelsu.ko"
    print(get_string("dl_lkm_downloading").format(asset=asset_pattern_regex))
    
    fetch_command = [
        "--repo", f"https://github.com/{const.KSU_APK_REPO}",
        "--tag", const.KSU_APK_TAG,
        "--release-asset", asset_pattern_regex,
        str(target_path.parent)
    ]
    
    try:
        _run_fetch_command(fetch_command)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(get_string("dl_lkm_download_fail").format(asset=asset_pattern_regex), file=sys.stderr)
        print(f"[!] {e}", file=sys.stderr)
        raise ToolError(get_string("dl_lkm_download_fail").format(asset=asset_pattern_regex))
    
    downloaded_files = list(target_path.parent.glob(f"android*-{kernel_version}_kernelsu.ko"))
    
    if not downloaded_files:
        raise ToolError(get_string("dl_lkm_download_fail").format(asset=asset_pattern_regex))
    
    downloaded_file = downloaded_files[0]
    shutil.move(downloaded_file, target_path)
    print(get_string("dl_lkm_download_ok"))

def install_base_tools(lang_code: str = "en"):
    i18n_load_lang(lang_code)
    
    print(get_string("dl_base_installing"))
    const.DOWNLOAD_DIR.mkdir(exist_ok=True)
    try:
        print(get_string("utils_check_deps"))
        req_path = const.BASE_DIR / "bin" / "requirements.txt"
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

if __name__ == "__main__":
    lang_code = "en" 
    if "--lang" in sys.argv:
        try:
            lang_code = sys.argv[sys.argv.index("--lang") + 1]
        except (IndexError, ValueError):
            pass 
    
    if len(sys.argv) > 1 and "install_base_tools" in sys.argv:
        install_base_tools(lang_code)