import json
import sys
from pathlib import Path
from typing import Any

BASE_DIR = Path(__file__).parent.parent.parent.resolve()
LTBOX_DIR = BASE_DIR / "bin" / "ltbox"
TOOLS_DIR = BASE_DIR / "bin" / "tools"
DOWNLOAD_DIR = TOOLS_DIR / "dl"
PYTHON_DIR = BASE_DIR / "bin" / "python3"

CONFIG_FILE = LTBOX_DIR / "config.json"

FN_BOOT = "boot.img"
FN_INIT_BOOT = "init_boot.img"
FN_VENDOR_BOOT = "vendor_boot.img"
FN_VBMETA = "vbmeta.img"
FN_VBMETA_SYSTEM = "vbmeta_system.img"
FN_DEVINFO = "devinfo.img"
FN_PERSIST = "persist.img"

FN_BOOT_BAK = "boot.bak.img"
FN_INIT_BOOT_BAK = "init_boot.bak.img"
FN_VBMETA_BAK = "vbmeta.bak.img"
FN_VENDOR_BOOT_BAK = "vendor_boot.bak.img"

FN_BOOT_ROOT = "boot.root.img"
FN_INIT_BOOT_ROOT = "init_boot.root.img"
FN_VBMETA_ROOT = "vbmeta.root.img"

FN_VENDOR_BOOT_PRC = "vendor_boot_prc.img"

_config = {}

def load_config() -> None:
    global _config
    if CONFIG_FILE.exists():
        try:
            with open(CONFIG_FILE, 'r', encoding='utf-8') as f:
                _config = json.load(f)
        except Exception as e:
            raise RuntimeError(f"[!] Critical Error: Failed to load config.json: {e}")
    else:
        raise RuntimeError(f"[!] Critical Error: Configuration file missing: {CONFIG_FILE}")

def _get_cfg(section: str, key: str, default: Any = None) -> Any:
    if not _config:
        load_config()
    try:
        return _config[section][key]
    except KeyError:
        if default is not None:
            return default
        raise RuntimeError(f"[!] Critical Error: Missing configuration key: [{section}][{key}]")

OUTPUT_DIR = BASE_DIR / "output"
OUTPUT_ROOT_DIR = BASE_DIR / "output_root"
OUTPUT_ROOT_LKM_DIR = BASE_DIR / "output_root_lkm"
OUTPUT_DP_DIR = BASE_DIR / "output_dp"
BACKUP_DIR = BASE_DIR / "backup"
WORK_DIR = BASE_DIR / "patch_work"

BACKUP_BOOT_DIR = BASE_DIR / "backup_boot"
BACKUP_INIT_BOOT_DIR = BASE_DIR / "backup_init_boot"
WORKING_BOOT_DIR = BASE_DIR / "working_boot"

OUTPUT_ANTI_ROLLBACK_DIR = BASE_DIR / "output_anti_rollback"

IMAGE_DIR = BASE_DIR / "image"
WORKING_DIR = BASE_DIR / "working"
OUTPUT_XML_DIR = BASE_DIR / "output_xml"

PYTHON_EXE = PYTHON_DIR / "python.exe"
ADB_EXE = DOWNLOAD_DIR / "adb.exe"
FASTBOOT_EXE = DOWNLOAD_DIR / "fastboot.exe"
AVBTOOL_PY = DOWNLOAD_DIR / "avbtool.py"
QSAHARASERVER_EXE = TOOLS_DIR / "Qsaharaserver.exe"
EDL_EXE = TOOLS_DIR / "fh_loader.exe"

MAGISKBOOT_REPO_URL = _get_cfg("magiskboot", "repo_url")
MAGISKBOOT_TAG = _get_cfg("magiskboot", "tag")

KSU_APK_REPO = _get_cfg("kernelsu", "apk_repo")
KSU_APK_TAG = _get_cfg("kernelsu", "apk_tag")
RELEASE_OWNER = _get_cfg("kernelsu", "release_owner")
RELEASE_REPO = _get_cfg("kernelsu", "release_repo")
RELEASE_TAG = _get_cfg("kernelsu", "release_tag")
REPO_URL = f"https://github.com/{RELEASE_OWNER}/{RELEASE_REPO}"
ANYKERNEL_ZIP_FILENAME = _get_cfg("kernelsu", "anykernel_zip")

EDL_LOADER_FILENAME = _get_cfg("edl", "loader_filename")
EDL_LOADER_FILE = IMAGE_DIR / EDL_LOADER_FILENAME 

FETCH_VERSION = _get_cfg("tools", "fetch_version")
FETCH_REPO_URL = _get_cfg("tools", "fetch_repo_url")
PLATFORM_TOOLS_ZIP_URL = _get_cfg("tools", "platform_tools_url")
AVB_ARCHIVE_URL = _get_cfg("tools", "avb_archive_url")

ROW_PATTERN_DOT = bytes.fromhex(_get_cfg("patterns", "row_dot"))
PRC_PATTERN_DOT = bytes.fromhex(_get_cfg("patterns", "prc_dot"))
ROW_PATTERN_I = bytes.fromhex(_get_cfg("patterns", "row_i"))
PRC_PATTERN_I = bytes.fromhex(_get_cfg("patterns", "prc_i"))

def _build_key_map() -> dict[str, Path]:
    if not _config:
        load_config()
    try:
        cfg_map = _config.get("key_map", {})
        return {key: DOWNLOAD_DIR / filename for key, filename in cfg_map.items()}
    except KeyError:
         raise RuntimeError(f"[!] Critical Error: Missing configuration section: [key_map]")

KEY_MAP = _build_key_map()

if not _config:
    load_config()
COUNTRY_CODES = _config.get("country_codes", {})
SORTED_COUNTRY_CODES = sorted(COUNTRY_CODES.items(), key=lambda item: item[1])