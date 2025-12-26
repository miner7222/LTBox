import os
import platform
import shutil
import subprocess
import sys
import zipfile
import tarfile
import re
from pathlib import Path
from typing import Dict, List, Optional

from . import constants as const
from . import utils
from .i18n import get_string, load_lang as i18n_load_lang
from .errors import ToolError

def download_resource(url: str, dest_path: Path) -> None:
    import urllib.request
    from urllib.error import URLError, HTTPError

    msg = get_string("dl_downloading").format(filename=dest_path.name)
    utils.ui.echo(msg)
    try:
        with urllib.request.urlopen(url) as response, open(dest_path, 'wb') as f:
            if response.status < 200 or response.status >= 300:
                 raise HTTPError(url, response.status, get_string("err_http_error").format(code=response.status), response.headers, None)
            shutil.copyfileobj(response, f)

        msg_success = get_string("dl_download_success").format(filename=dest_path.name)
        utils.ui.echo(msg_success)
    except (HTTPError, URLError, OSError) as e:
        msg_err = get_string("dl_download_failed").format(url=url, error=e)
        utils.ui.error(msg_err)
        if dest_path.exists():
            dest_path.unlink()
        raise ToolError(get_string("dl_err_download_tool").format(name=dest_path.name))

def extract_archive_files(archive_path: Path, extract_map: Dict[str, Path]) -> None:
    msg = get_string("dl_extracting").format(filename=archive_path.name)
    utils.ui.echo(msg)
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
                            utils.ui.echo(get_string("dl_extracted_file").format(filename=target_path.name))
        else:
            with zipfile.ZipFile(archive_path, 'r') as zf:
                for member in zf.infolist():
                    if member.filename in extract_map:
                        target_path = extract_map[member.name]
                        with zf.open(member) as source, open(target_path, "wb") as target:
                            shutil.copyfileobj(source, target)
                        utils.ui.echo(get_string("dl_extracted_file").format(filename=target_path.name))
                        
    except (zipfile.BadZipFile, tarfile.TarError, OSError, IOError) as e:
        msg_err = get_string("dl_extract_failed").format(filename=archive_path.name, error=e)
        utils.ui.error(msg_err)
        raise ToolError(get_string("dl_err_extract_tool").format(name=archive_path.name))

def _download_github_asset(repo_url: str, tag: str, asset_pattern: str, dest_dir: Path) -> Path:
    import requests
    from requests.exceptions import RequestException
    
    if "github.com/" in repo_url:
        owner_repo = repo_url.split("github.com/")[-1]
    else:
        owner_repo = repo_url

    api_url = f"https://api.github.com/repos/{owner_repo}/releases/tags/{tag}"
    
    try:
        response = requests.get(api_url)
        response.raise_for_status()
        release_data = response.json()
        
        target_asset = None
        for asset in release_data.get('assets', []):
            if re.match(asset_pattern, asset['name']):
                target_asset = asset
                break
        
        if not target_asset:
            raise ToolError(get_string("dl_err_download_tool").format(name=asset_pattern))

        download_url = target_asset['browser_download_url']
        filename = target_asset['name']
        dest_path = dest_dir / filename

        utils.ui.echo(get_string("dl_downloading").format(filename=filename))
        
        with requests.get(download_url, stream=True) as r:
            r.raise_for_status()
            with open(dest_path, 'wb') as f:
                shutil.copyfileobj(r.raw, f)
                
        return dest_path

    except RequestException as e:
        utils.ui.error(get_string("dl_err_check_network"))
        raise ToolError(get_string("dl_github_failed").format(e=e))

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

    utils.ui.echo(get_string("dl_tool_not_found").format(tool_name=tool_exe.name))
    const.DOWNLOAD_DIR.mkdir(exist_ok=True)
    
    arch = platform.machine()
    asset_pattern = asset_patterns.get(arch)
    if not asset_pattern:
        msg = get_string("dl_unsupported_arch").format(arch=arch, tool_name=tool_name)
        utils.ui.error(msg)
        raise ToolError(msg)

    msg = get_string("dl_detect_arch").format(arch=arch, pattern=asset_pattern)
    utils.ui.echo(msg)

    try:
        downloaded_zip_path = _download_github_asset(repo_url, tag, asset_pattern, const.DOWNLOAD_DIR)

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
        utils.ui.echo(get_string("dl_tool_success").format(tool_name=tool_name))
        return tool_exe

    except (FileNotFoundError, zipfile.BadZipFile, OSError, ToolError) as e:
        msg_err = get_string("dl_tool_failed").format(tool_name=tool_name, error=e)
        utils.ui.error(msg_err)
        raise ToolError(msg_err)

def ensure_platform_tools() -> None:
    if const.ADB_EXE.exists() and const.FASTBOOT_EXE.exists():
        return
    
    utils.ui.echo(get_string("dl_platform_not_found"))
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
        utils.ui.echo(get_string("dl_platform_success"))
        
    except (zipfile.BadZipFile, OSError, IOError) as e:
        msg_err = get_string("dl_platform_failed").format(error=e)
        utils.ui.error(msg_err)
        if temp_zip_path.exists():
            temp_zip_path.unlink()
        raise ToolError(msg_err)

def ensure_avb_tools() -> None:
    key1 = const.DOWNLOAD_DIR / "testkey_rsa4096.pem"
    key2 = const.DOWNLOAD_DIR / "testkey_rsa2048.pem"
    
    if const.AVBTOOL_PY.exists() and key1.exists() and key2.exists():
        return

    utils.ui.echo(get_string("dl_avb_not_found"))
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
    utils.ui.echo(get_string("dl_avb_ready"))

def ensure_magiskboot() -> Path:
    asset_patterns = {
        'AMD64': "magiskboot-.*-windows-.*-x86_64-standalone\\.zip",
    }
    
    return _ensure_tool_from_github_release(
        tool_name="magiskboot",
        exe_name_in_zip="magiskboot.exe",
        repo_url=const.MAGISKBOOT_REPO_URL,
        tag=const.MAGISKBOOT_TAG,
        asset_patterns=asset_patterns
    )

def get_gki_kernel(kernel_version: str, work_dir: Path) -> Path:
    utils.ui.echo(get_string("dl_gki_downloading"))
    asset_pattern = f".*{kernel_version}.*Normal-AnyKernel3.zip"
    
    try:
        downloaded_zip = _download_github_asset(const.REPO_URL, const.RELEASE_TAG, asset_pattern, work_dir)
        
        anykernel_zip = work_dir / const.ANYKERNEL_ZIP_FILENAME
        shutil.move(downloaded_zip, anykernel_zip)
        utils.ui.echo(get_string("dl_gki_download_ok"))

        utils.ui.echo(get_string("dl_gki_extracting"))
        extracted_kernel_dir = work_dir / "extracted_kernel"
        with zipfile.ZipFile(anykernel_zip, 'r') as zip_ref:
            zip_ref.extractall(extracted_kernel_dir)
        
        kernel_image = extracted_kernel_dir / "Image"
        if not kernel_image.exists():
            utils.ui.echo(get_string("dl_gki_image_missing"))
            raise ToolError(get_string("dl_gki_image_missing"))
        utils.ui.echo(get_string("dl_gki_extract_ok"))
        return kernel_image
    except Exception as e:
        utils.ui.echo(get_string("dl_gki_download_fail").format(version=kernel_version))
        raise ToolError(f"{e}")

def ensure_openssl() -> None:
    openssl_exe = const.DOWNLOAD_DIR / "openssl.exe"
    if openssl_exe.exists():
        return

    utils.ui.echo(get_string("dl_downloading").format(filename="OpenSSL"))

    url = const.CONF._get_val("tools", "openssl_url")
    temp_zip = const.DOWNLOAD_DIR / "openssl.zip"
    
    import requests
    from requests.exceptions import RequestException

    try:
        with requests.get(url, stream=True) as r:
            r.raise_for_status()
            with open(temp_zip, 'wb') as f:
                shutil.copyfileobj(r.raw, f)

        with zipfile.ZipFile(temp_zip, 'r') as zf:
            for member in zf.infolist():
                if member.is_dir():
                    continue

                if "x64/bin/" in member.filename:
                    filename = Path(member.filename).name
                    if not filename:
                        continue
                        
                    target_path = const.DOWNLOAD_DIR / filename
                    
                    with zf.open(member) as source, open(target_path, "wb") as target:
                        shutil.copyfileobj(source, target)
        
        utils.ui.echo(get_string("dl_tool_success").format(tool_name="OpenSSL"))
        
    except (RequestException, zipfile.BadZipFile, OSError) as e:
        utils.ui.error(get_string("dl_err_openssl_download").format(e=e))
        if temp_zip.exists():
            temp_zip.unlink()
        raise ToolError(get_string("dl_err_openssl_generic"))
    finally:
        if temp_zip.exists():
            temp_zip.unlink()

def download_ksu_apk(target_dir: Path) -> None:
    utils.ui.echo(get_string("dl_ksu_downloading"))
    if list(target_dir.glob("*spoofed*.apk")):
        utils.ui.echo(get_string("dl_ksu_exists"))
    else:
        try:
            _download_github_asset(f"https://github.com/{const.KSU_APK_REPO}", const.KSU_APK_TAG, ".*spoofed.*\\.apk", target_dir)
            utils.ui.echo(get_string("dl_ksu_success"))
        except ToolError as e:utils.ui.echo(get_string("dl_err_ksu_dl_spoof"))
        try:
            _download_github_asset(f"https://github.com/{const.KSU_APK_REPO}", const.KSU_APK_TAG, ".*\\.apk", target_dir)
            utils.ui.echo(get_string("dl_ksu_success"))
        except ToolError as e:
             utils.ui.error(get_string("dl_err_ksu_download").format(e=e))

def download_ksuinit(target_path: Path) -> None:
    if target_path.exists():
        target_path.unlink()
    
    url = f"https://github.com/{const.KSU_APK_REPO}/raw/refs/tags/{const.KSU_APK_TAG}/userspace/ksud/bin/aarch64/ksuinit"
    
    import requests
    from requests.exceptions import RequestException

    msg = get_string("dl_downloading").format(filename="ksuinit")
    utils.ui.echo(msg)
    try:
        with requests.get(url, stream=True, allow_redirects=True) as response:
            response.raise_for_status()
            with open(target_path, 'wb') as f:
                for chunk in response.iter_content(chunk_size=8192):
                    f.write(chunk)
        msg_success = get_string("dl_download_success").format(filename="ksuinit")
        utils.ui.echo(msg_success)
    
    except (RequestException, OSError) as e:
        msg_err = get_string("dl_download_failed").format(url=url, error=e)
        utils.ui.error(msg_err)
        if target_path.exists():
            target_path.unlink()
        raise ToolError(get_string("dl_err_download_tool").format(name="ksuinit"))

def download_sukisu_manager(target_dir: Path) -> None:
    msg_downloading = get_string("dl_ksu_downloading")
    msg_downloading = msg_downloading.replace("KernelSU Next", "SukiSU Ultra")
    utils.ui.echo(msg_downloading)
    if list(target_dir.glob("SukiSU*.apk")):
        msg_exists = get_string("dl_ksu_exists")
        msg_exists = msg_exists.replace("KernelSU Next", "SukiSU Ultra")
        utils.ui.echo(msg_exists)
        return

    repo = const.SUKISU_REPO
    workflow = const.SUKISU_WORKFLOW
    url = f"https://nightly.link/{repo}/actions/runs/{workflow}/Spoofed-Manager.zip"
    temp_zip = target_dir / "temp_manager.zip"

    try:
        download_resource(url, temp_zip)
        
        with zipfile.ZipFile(temp_zip, 'r') as zf:
            for member in zf.infolist():
                if member.filename.startswith("SukiSU") and member.filename.endswith(".apk"):
                    with zf.open(member) as source:
                        target = target_dir / Path(member.filename).name
                        with open(target, "wb") as t:
                            shutil.copyfileobj(source, t)
                    msg_success = get_string("dl_ksu_success")
                    msg_success = msg_success.replace("KernelSU Next", "SukiSU Ultra")
                    utils.ui.echo(msg_success)
                    break
    except Exception as e:
        msg_err_download = get_string("dl_err_ksu_download")
        msg_err_download = msg_downloading.replace("KernelSU Next", "SukiSU Ultra")
        utils.ui.error(msg_err_download.format(e=e))
    finally:
        if temp_zip.exists():
            try:
                temp_zip.unlink()
            except OSError:
                pass

def download_sukisu_init(target_path: Path) -> None:
    if target_path.exists():
        target_path.unlink()

    repo = const.SUKISU_REPO
    workflow = const.SUKISU_WORKFLOW
    url = f"https://nightly.link/{repo}/actions/runs/{workflow}/ksuinit-aarch64-linux-android.zip"
    temp_zip = target_path.parent / "temp_init.zip"

    msg = get_string("dl_downloading").format(filename="ksuinit")
    utils.ui.echo(msg)

    try:
        download_resource(url, temp_zip)
        
        found = False
        with zipfile.ZipFile(temp_zip, 'r') as zf:
            for member in zf.infolist():
                if member.filename.endswith("ksuinit") and "release" in member.filename:
                    with zf.open(member) as source, open(target_path, "wb") as target:
                        shutil.copyfileobj(source, target)
                    found = True
                    break
        
        if found:
            utils.ui.echo(get_string("dl_download_success").format(filename="ksuinit"))
        else:
            raise ToolError("ksuinit not found in archive")

    except Exception as e:
        utils.ui.error(get_string("dl_download_failed").format(url=url, error=e))
        raise ToolError(get_string("dl_err_download_tool").format(name="ksuinit"))
    finally:
        if temp_zip.exists():
            temp_zip.unlink()

def get_sukisu_lkm(target_path: Path, kernel_version: str) -> None:
    if target_path.exists():
        target_path.unlink()
        
    if not kernel_version:
        raise ToolError("Kernel version is required for SukiSU LKM download")

    major_minor = ".".join(kernel_version.split(".")[:2])

    mapping = {
        "5.10": "android12-5.10",
        "5.15": "android13-5.15",
        "6.1":  "android14-6.1",
        "6.6":  "android15-6.6",
        "6.12": "android16-6.12"
    }
    
    mapped_name = mapping.get(major_minor)
    
    if not mapped_name:
         utils.ui.echo(f"Warning: No hardcoded mapping found for Kernel {major_minor}. Defaulting to android12-5.10 format fallback...")
         mapped_name = f"android12-{major_minor}"

    repo = const.SUKISU_REPO
    workflow = const.SUKISU_WORKFLOW
    url = f"https://nightly.link/{repo}/actions/runs/{workflow}/{mapped_name}-lkm.zip"
    temp_zip = target_path.parent / "temp_lkm.zip"

    utils.ui.echo(get_string("dl_lkm_downloading").format(asset=f"{mapped_name}-lkm"))

    try:
        download_resource(url, temp_zip)
        
        found = False
        with zipfile.ZipFile(temp_zip, 'r') as zf:
            for member in zf.infolist():
                if member.filename.endswith("_kernelsu.ko"):
                    with zf.open(member) as source, open(target_path, "wb") as target:
                        shutil.copyfileobj(source, target)
                    found = True
                    break
        
        if found:
            utils.ui.echo(get_string("dl_lkm_download_ok"))
        else:
            raise ToolError("kernelsu.ko not found in archive")

    except Exception as e:
        utils.ui.error(get_string("dl_lkm_download_fail").format(asset=mapped_name))
        utils.ui.error(f"[!] {e}")
        raise ToolError(str(e))
    finally:
        if temp_zip.exists():
            temp_zip.unlink()

def get_lkm_kernel(target_path: Path, kernel_version: str) -> None:
    if target_path.exists():
        target_path.unlink()
        
    if not kernel_version:
        raise ToolError("Kernel version is required for LKM download")

    utils.ui.echo(get_string("dl_lkm_kver_found").format(ver=kernel_version))
    
    asset_pattern_regex = f"android.*-{kernel_version}_kernelsu.ko"
    utils.ui.echo(get_string("dl_lkm_downloading").format(asset=asset_pattern_regex))
    
    try:
        downloaded_file = _download_github_asset(f"https://github.com/{const.KSU_APK_REPO}", const.KSU_APK_TAG, asset_pattern_regex, target_path.parent)
        shutil.move(downloaded_file, target_path)
        utils.ui.echo(get_string("dl_lkm_download_ok"))
    except (ToolError, OSError) as e:
        utils.ui.error(get_string("dl_lkm_download_fail").format(asset=asset_pattern_regex))
        utils.ui.error(f"[!] {e}")
        raise ToolError(str(e))

def install_base_tools(lang_code: str = "en"):
    i18n_load_lang(lang_code)
    
    utils.ui.echo(get_string("dl_base_installing"))
    const.DOWNLOAD_DIR.mkdir(exist_ok=True)
    try:
        utils.ui.echo(get_string("utils_check_deps"))
        req_path = const.BASE_DIR / "bin" / "requirements.txt"
        subprocess.run(
            [str(const.PYTHON_EXE), "-m", "pip", "install", "-r", str(req_path)],
            check=True
        )
        
        ensure_platform_tools()
        ensure_avb_tools()
        ensure_openssl()
        utils.ui.echo(get_string("dl_base_complete"))
    except Exception as e:
        msg = get_string("dl_base_error").format(error=e)
        utils.ui.error(msg)
        input(get_string("press_enter_to_exit"))
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