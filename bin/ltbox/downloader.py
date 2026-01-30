import platform
import re
import shutil
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path
from typing import Dict, Optional

import requests

from . import constants as const
from . import net, utils
from .errors import ToolError
from .i18n import get_string
from .i18n import load_lang as i18n_load_lang


def _get_owner_repo(repo_url: str) -> str:
    if "github.com/" in repo_url:
        return repo_url.split("github.com/")[-1]
    return repo_url


def _extract_zip_member(
    zip_file: zipfile.ZipFile, member: zipfile.ZipInfo, target_path: Path
) -> None:
    with zip_file.open(member) as source, open(target_path, "wb") as target:
        shutil.copyfileobj(source, target)


def download_resource(url: str, dest_path: Path, show_progress: bool = True) -> None:
    msg = get_string("dl_downloading").format(filename=dest_path.name)
    utils.ui.echo(msg)
    try:
        with net.request_with_retries("GET", url, stream=True) as response:
            downloaded = 0

            with open(dest_path, "wb") as f:
                for chunk in response.iter_content(chunk_size=8192):
                    if chunk:
                        f.write(chunk)
                        downloaded += len(chunk)

        msg_success = get_string("dl_download_success").format(filename=dest_path.name)
        utils.ui.echo(msg_success)
    except (requests.RequestException, OSError) as e:
        msg_err = get_string("dl_download_failed").format(url=url, error=e)
        utils.ui.error(msg_err)
        if dest_path.exists():
            dest_path.unlink()
        raise ToolError(get_string("dl_err_download_tool").format(name=dest_path.name))


def extract_archive_files(archive_path: Path, extract_map: Dict[str, Path]) -> None:
    msg = get_string("dl_extracting").format(filename=archive_path.name)
    utils.ui.echo(msg)
    try:
        is_tar = archive_path.suffix == ".gz" or archive_path.suffix == ".tar"

        if is_tar:
            with tarfile.open(archive_path, "r:*") as tf:
                for member in tf:
                    if member.name in extract_map:
                        target_path = extract_map[member.name]
                        f = tf.extractfile(member)
                        if f:
                            with open(target_path, "wb") as target:
                                shutil.copyfileobj(f, target)
                            utils.ui.echo(
                                get_string("dl_extracted_file").format(
                                    filename=target_path.name
                                )
                            )
        else:
            with zipfile.ZipFile(archive_path, "r") as zf:
                for zip_member in zf.infolist():
                    if zip_member.filename in extract_map:
                        target_path = extract_map[zip_member.filename]
                        _extract_zip_member(zf, zip_member, target_path)
                        utils.ui.echo(
                            get_string("dl_extracted_file").format(
                                filename=target_path.name
                            )
                        )

    except (zipfile.BadZipFile, tarfile.TarError, OSError, IOError) as e:
        msg_err = get_string("dl_extract_failed").format(
            filename=archive_path.name, error=e
        )
        utils.ui.error(msg_err)
        raise ToolError(
            get_string("dl_err_extract_tool").format(name=archive_path.name)
        )


def _download_github_asset(
    repo_url: str, tag: str, asset_pattern: str, dest_dir: Path
) -> Path:
    import requests
    from requests.exceptions import RequestException

    owner_repo = _get_owner_repo(repo_url)

    if not tag or tag.lower() == "latest":
        api_url = f"https://api.github.com/repos/{owner_repo}/releases/latest"
    else:
        api_url = f"https://api.github.com/repos/{owner_repo}/releases/tags/{tag}"

    try:
        response = requests.get(api_url)
        response.raise_for_status()
        release_data = response.json()

        target_asset = None
        for asset in release_data.get("assets", []):
            if re.match(asset_pattern, asset["name"]):
                target_asset = asset
                break

        if not target_asset:
            raise ToolError(
                get_string("dl_err_download_tool").format(name=asset_pattern)
            )

        download_url = target_asset["browser_download_url"]
        filename = target_asset["name"]
        dest_path = dest_dir / filename

        download_resource(download_url, dest_path)
        return dest_path

    except RequestException as e:
        utils.ui.error(get_string("dl_err_check_network"))
        raise ToolError(get_string("dl_github_failed").format(e=e))


def _get_latest_release_tag(owner_repo: str) -> str:
    api_url = f"https://api.github.com/repos/{owner_repo}/releases/latest"
    try:
        response = requests.get(api_url, timeout=15)
        response.raise_for_status()
        release_data = response.json()
    except requests.RequestException as e:
        utils.ui.error(get_string("dl_err_check_network"))
        raise ToolError(get_string("dl_github_failed").format(e=e))

    tag_name = release_data.get("tag_name")
    if not tag_name:
        raise ToolError("Failed to resolve latest KernelSU Next tag.")
    return tag_name


def _resolve_release_tag(owner_repo: str, tag: Optional[str]) -> str:
    if not tag or tag.lower() == "latest":
        return _get_latest_release_tag(owner_repo)
    return tag


def _select_workflow_run_for_tag(runs: list[dict], tag: str) -> Optional[dict]:
    for run in runs:
        head_branch = run.get("head_branch") or ""
        if head_branch == tag or head_branch == f"refs/tags/{tag}":
            return run
    for run in runs:
        head_branch = run.get("head_branch") or ""
        if head_branch.endswith(f"/{tag}"):
            return run
    return None


def _get_workflow_run_id_for_tag(owner_repo: str, tag: str) -> str:
    api_url = f"https://api.github.com/repos/{owner_repo}/actions/runs"
    params = {"per_page": 30, "status": "completed", "branch": tag}
    try:
        response = requests.get(api_url, params=params, timeout=15)
        response.raise_for_status()
        data = response.json()
        run = _select_workflow_run_for_tag(data.get("workflow_runs", []), tag)
        if run:
            return str(run["id"])

        response = requests.get(api_url, params={"per_page": 50}, timeout=15)
        response.raise_for_status()
        data = response.json()
        run = _select_workflow_run_for_tag(data.get("workflow_runs", []), tag)
        if run:
            return str(run["id"])
    except requests.RequestException as e:
        utils.ui.error(get_string("dl_err_check_network"))
        raise ToolError(get_string("dl_github_failed").format(e=e))

    raise ToolError(f"Failed to find workflow run for tag {tag}.")


def _ensure_tool_from_github_release(
    tool_name: str,
    exe_name_in_zip: str,
    repo_url: str,
    tag: str,
    asset_patterns: Dict[str, str],
) -> Path:
    tool_exe = const.DOWNLOAD_DIR / f"{tool_name}.exe"
    if tool_exe.exists():
        return tool_exe

    utils.ui.echo(get_string("dl_tool_not_found").format(tool_name=tool_exe.name))
    const.DOWNLOAD_DIR.mkdir(parents=True, exist_ok=True)

    arch = platform.machine()
    asset_pattern = asset_patterns.get(arch)
    if not asset_pattern:
        msg = get_string("dl_unsupported_arch").format(arch=arch, tool_name=tool_name)
        utils.ui.error(msg)
        raise ToolError(msg)

    msg = get_string("dl_detect_arch").format(arch=arch, pattern=asset_pattern)
    utils.ui.echo(msg)

    try:
        downloaded_zip_path = _download_github_asset(
            repo_url, tag, asset_pattern, const.DOWNLOAD_DIR
        )

        with zipfile.ZipFile(downloaded_zip_path, "r") as zip_ref:
            exe_info = None
            for member in zip_ref.infolist():
                if member.filename.endswith(exe_name_in_zip):
                    exe_info = member
                    break

            if not exe_info:
                raise FileNotFoundError(
                    get_string("dl_err_exe_in_zip_not_found").format(
                        exe_name=exe_name_in_zip, zip_name=downloaded_zip_path.name
                    )
                )

            extracted_path = const.DOWNLOAD_DIR / Path(exe_info.filename).name
            _extract_zip_member(zip_ref, exe_info, extracted_path)

            if extracted_path != tool_exe:
                shutil.move(extracted_path, tool_exe)

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
    const.DOWNLOAD_DIR.mkdir(parents=True, exist_ok=True)
    temp_zip_path = const.DOWNLOAD_DIR / "platform-tools.zip"

    settings = const.load_settings_raw()
    url = settings.get("tools", {}).get("platform_tools_url")
    download_resource(url, temp_zip_path)

    try:
        with zipfile.ZipFile(temp_zip_path) as zf:
            for member in zf.infolist():
                if member.is_dir():
                    continue

                if re.match(r"^platform-tools/[^/]+$", member.filename):
                    file_name = Path(member.filename).name
                    target_path = const.DOWNLOAD_DIR / file_name
                    _extract_zip_member(zf, member, target_path)

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
    const.DOWNLOAD_DIR.mkdir(parents=True, exist_ok=True)
    temp_tar_path = const.DOWNLOAD_DIR / "avb.tar.gz"

    settings = const.load_settings_raw()
    url = settings.get("tools", {}).get("avb_archive_url")
    download_resource(url, temp_tar_path)

    files_to_extract = {
        "avbtool.py": const.AVBTOOL_PY,
        "test/data/testkey_rsa4096.pem": key1,
        "test/data/testkey_rsa2048.pem": key2,
    }

    extract_archive_files(temp_tar_path, files_to_extract)
    temp_tar_path.unlink()
    utils.ui.echo(get_string("dl_avb_ready"))


def ensure_openssl() -> None:
    openssl_exe = const.DOWNLOAD_DIR / "openssl.exe"
    if openssl_exe.exists():
        return

    utils.ui.echo(get_string("dl_downloading").format(filename="OpenSSL"))

    settings = const.load_settings_raw()
    url = settings.get("tools", {}).get("openssl_url")
    temp_zip = const.DOWNLOAD_DIR / "openssl.zip"

    try:
        download_resource(url, temp_zip)

        with zipfile.ZipFile(temp_zip, "r") as zf:
            for member in zf.infolist():
                if member.is_dir():
                    continue

                if "x64/bin/" in member.filename:
                    filename = Path(member.filename).name
                    if not filename:
                        continue

                    target_path = const.DOWNLOAD_DIR / filename
                    _extract_zip_member(zf, member, target_path)

        utils.ui.echo(get_string("dl_tool_success").format(tool_name="OpenSSL"))

    except (ToolError, zipfile.BadZipFile, OSError) as e:
        utils.ui.error(get_string("dl_err_openssl_download").format(e=e))
        raise ToolError(get_string("dl_err_openssl_generic"))
    finally:
        if temp_zip.exists():
            temp_zip.unlink()


def ensure_magiskboot() -> Path:
    asset_patterns = {
        "AMD64": "magiskboot-.*-windows-.*-x86_64-standalone\\.zip",
    }

    settings = const.load_settings_raw()
    mb_config = settings.get("magiskboot", {})

    return _ensure_tool_from_github_release(
        tool_name="magiskboot",
        exe_name_in_zip="magiskboot.exe",
        repo_url=mb_config.get("repo"),
        tag=mb_config.get("tag"),
        asset_patterns=asset_patterns,
    )


def get_gki_kernel(kernel_version: str, work_dir: Path) -> Path:
    utils.ui.echo(get_string("dl_gki_downloading"))

    try:
        tag = const.CONF._get_val("wildkernels", "tag", default="latest")
        owner = const.CONF._get_val("wildkernels", "owner")
        repo = const.CONF._get_val("wildkernels", "repo")
    except RuntimeError:
        tag = "latest"
        owner = const.RELEASE_OWNER
        repo = const.RELEASE_REPO

    if not tag:
        tag = "latest"
    repo_ref = f"{owner}/{repo}"

    asset_pattern = f"{re.escape(kernel_version)}.*Normal.*AnyKernel3\\.zip"

    try:
        downloaded_zip = _download_github_asset(repo_ref, tag, asset_pattern, work_dir)

        anykernel_zip = work_dir / const.ANYKERNEL_ZIP_FILENAME
        if anykernel_zip.exists():
            anykernel_zip.unlink()
        shutil.move(downloaded_zip, anykernel_zip)

        utils.ui.echo(get_string("dl_gki_download_ok"))

        utils.ui.echo(get_string("dl_gki_extracting"))
        extracted_kernel_dir = work_dir / "extracted_kernel"
        if extracted_kernel_dir.exists():
            shutil.rmtree(extracted_kernel_dir)

        with zipfile.ZipFile(anykernel_zip, "r") as zip_ref:
            zip_ref.extractall(extracted_kernel_dir)

        kernel_image = extracted_kernel_dir / "Image"
        if not kernel_image.exists():
            utils.ui.echo(get_string("dl_gki_image_missing"))
            raise ToolError(get_string("dl_gki_image_missing"))

        utils.ui.echo(get_string("dl_gki_extract_ok"))
        return kernel_image

    except Exception as e:
        utils.ui.echo(get_string("dl_gki_download_fail").format(version=tag))
        raise ToolError(f"{e}")


def download_nightly_artifacts(
    repo: str, workflow_id: str, manager_name: str, mapped_name: str, target_dir: Path
):
    base_url = f"https://nightly.link/{repo}/actions/runs/{workflow_id}"

    manager_url = f"{base_url}/{manager_name}"
    lkm_url = f"{base_url}/{mapped_name}-lkm.zip"

    manager_zip = target_dir / manager_name
    ksuinit_dest = target_dir / "ksuinit"
    lkm_dest = target_dir / "lkm.zip"

    utils.ui.info(f"Fetching artifacts from Workflow {workflow_id}...")

    try:
        download_resource(manager_url, manager_zip)

        ksuinit_variants = ["ksuinit", "ksuinit-aarch64-linux-android"]
        ksuinit_downloaded = False

        for variant in ksuinit_variants:
            ksuinit_url = f"{base_url}/{variant}.zip"
            temp_ksuinit_zip = target_dir / f"temp_{variant}.zip"
            try:
                download_resource(ksuinit_url, temp_ksuinit_zip)

                with zipfile.ZipFile(temp_ksuinit_zip, "r") as zf:
                    for member in zf.namelist():
                        if member.endswith("ksuinit"):
                            with zf.open(member) as src, open(
                                ksuinit_dest, "wb"
                            ) as dst:
                                shutil.copyfileobj(src, dst)
                            ksuinit_downloaded = True
                            break
                temp_ksuinit_zip.unlink()

                if ksuinit_downloaded:
                    break
            except Exception:
                if temp_ksuinit_zip.exists():
                    temp_ksuinit_zip.unlink()
                continue

        if not ksuinit_downloaded:
            raise ToolError("Failed to download ksuinit (tried variants)")

        download_resource(lkm_url, lkm_dest)

        utils.ui.echo(
            get_string("dl_download_success").format(filename="All Artifacts")
        )

    except Exception as e:
        if manager_zip.exists():
            manager_zip.unlink()
        if ksuinit_dest.exists():
            ksuinit_dest.unlink()
        if lkm_dest.exists():
            lkm_dest.unlink()
        raise e


def download_ksu_manager_release(target_dir: Path) -> None:
    utils.ui.echo(get_string("dl_ksu_downloading"))

    target_file = target_dir / "manager.apk"
    if target_file.exists():
        target_file.unlink()

    downloaded_path = None
    try:
        downloaded_path = _download_github_asset(
            f"https://github.com/{const.KSU_APK_REPO}",
            const.KSU_APK_TAG,
            ".*spoofed.*\\.apk",
            target_dir,
        )
    except ToolError:
        try:
            downloaded_path = _download_github_asset(
                f"https://github.com/{const.KSU_APK_REPO}",
                const.KSU_APK_TAG,
                ".*\\.apk",
                target_dir,
            )
        except ToolError as e:
            utils.ui.error(get_string("dl_err_ksu_download").format(e=e))
            return

    if downloaded_path and downloaded_path.exists():
        shutil.move(downloaded_path, target_file)
        utils.ui.echo(get_string("dl_ksu_success"))


def download_magisk_apk(target_dir: Path) -> Path:
    utils.ui.echo(get_string("dl_magisk_downloading"))

    target_file = target_dir / "magisk.apk"
    if target_file.exists():
        target_file.unlink()

    downloaded_path = _download_github_asset(
        f"https://github.com/{const.MAGISK_REPO}",
        const.MAGISK_TAG,
        r"Magisk.*\.apk",
        target_dir,
    )
    shutil.move(downloaded_path, target_file)
    utils.ui.echo(get_string("dl_magisk_success"))
    return target_file


def extract_magisk_libs(apk_path: Path, target_dir: Path) -> None:
    utils.ui.echo(get_string("dl_magisk_extracting"))
    lib_map = {
        "libmagiskinit.so": "magiskinit",
        "libmagisk.so": "magisk",
        "libinit-ld.so": "init-ld",
    }
    asset_map = {
        "assets/stub.apk": "stub.apk",
    }
    missing = []

    with zipfile.ZipFile(apk_path, "r") as zf:
        for source_name, target_name in lib_map.items():
            source_path = f"lib/arm64-v8a/{source_name}"
            try:
                member = zf.getinfo(source_path)
            except KeyError:
                missing.append(source_name)
                continue

            target_path = target_dir / target_name
            _extract_zip_member(zf, member, target_path)

        for source_name, target_name in asset_map.items():
            try:
                member = zf.getinfo(source_name)
            except KeyError:
                missing.append(source_name)
                continue

            target_path = target_dir / target_name
            _extract_zip_member(zf, member, target_path)

    if missing:
        raise ToolError(
            get_string("dl_magisk_lib_missing").format(files=", ".join(missing))
        )

    utils.ui.echo(get_string("dl_magisk_extract_ok"))


def download_ksuinit_release(target_path: Path) -> None:
    if target_path.exists():
        target_path.unlink()

    owner_repo = _get_owner_repo(f"https://github.com/{const.KSU_APK_REPO}")
    tag = _resolve_release_tag(owner_repo, const.KSU_APK_TAG)
    workflow_id = _get_workflow_run_id_for_tag(owner_repo, tag)
    base_url = f"https://nightly.link/{owner_repo}/actions/runs/{workflow_id}"
    temp_zip = target_path.parent / "ksuinit.zip"

    try:
        download_resource(f"{base_url}/ksuinit.zip", temp_zip)
        with zipfile.ZipFile(temp_zip, "r") as zf:
            ksuinit_member = None
            for member in zf.namelist():
                if member.endswith("ksuinit"):
                    ksuinit_member = member
                    break
            if not ksuinit_member:
                raise ToolError("ksuinit not found in workflow artifact.")

            with zf.open(ksuinit_member) as src, open(target_path, "wb") as dst:
                shutil.copyfileobj(src, dst)
    finally:
        if temp_zip.exists():
            temp_zip.unlink()


def get_lkm_kernel_release(target_path: Path, kernel_version: str) -> None:
    if target_path.exists():
        target_path.unlink()

    if not kernel_version:
        raise ToolError(get_string("err_req_kernel_ver_lkm"))

    utils.ui.echo(get_string("dl_lkm_kver_found").format(ver=kernel_version))

    asset_pattern_regex = f"android.*-{kernel_version}_kernelsu.ko"
    utils.ui.echo(get_string("dl_lkm_downloading").format(asset=asset_pattern_regex))

    try:
        downloaded_file = _download_github_asset(
            f"https://github.com/{const.KSU_APK_REPO}",
            const.KSU_APK_TAG,
            asset_pattern_regex,
            target_path.parent,
        )
        shutil.move(downloaded_file, target_path)
        utils.ui.echo(get_string("dl_lkm_download_ok"))
    except (ToolError, OSError) as e:
        utils.ui.error(
            get_string("dl_lkm_download_fail").format(asset=asset_pattern_regex)
        )
        raise ToolError(str(e))


def install_base_tools(lang_code: str = "en"):
    i18n_load_lang(lang_code)

    utils.ui.echo(get_string("dl_base_installing"))
    const.DOWNLOAD_DIR.mkdir(parents=True, exist_ok=True)
    try:
        utils.ui.echo(get_string("utils_check_deps"))
        req_path = const.BASE_DIR / "bin" / "requirements.txt"
        if req_path.exists():
            subprocess.run(
                [str(const.PYTHON_EXE), "-m", "pip", "install", "-r", str(req_path)],
                check=True,
            )

        ensure_platform_tools()
        ensure_avb_tools()
        ensure_openssl()
        ensure_magiskboot()

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
