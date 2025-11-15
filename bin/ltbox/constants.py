import json
import sys
from pathlib import Path

BASE_DIR = Path(__file__).parent.parent.parent.resolve()
LTBOX_DIR = BASE_DIR / "bin" / "ltbox"
TOOLS_DIR = BASE_DIR / "bin" / "tools"
DOWNLOAD_DIR = TOOLS_DIR / "dl"
PYTHON_DIR = BASE_DIR / "bin" / "python3"

CONFIG_FILE = LTBOX_DIR / "config.json"
_config = {}

def load_config():
    global _config
    if CONFIG_FILE.exists():
        try:
            with open(CONFIG_FILE, 'r', encoding='utf-8') as f:
                _config = json.load(f)
        except Exception as e:
            raise RuntimeError(f"[!] Critical Error: Failed to load config.json: {e}")
    else:
        raise RuntimeError(f"[!] Critical Error: Configuration file missing: {CONFIG_FILE}")

def _get_cfg(section: str, key: str) -> str:
    if not _config:
        load_config()
    try:
        return _config[section][key]
    except KeyError:
        raise RuntimeError(f"[!] Critical Error: Missing configuration key: [{section}][{key}]")

OUTPUT_DIR = BASE_DIR / "output"
OUTPUT_ROOT_DIR = BASE_DIR / "output_root"
OUTPUT_DP_DIR = BASE_DIR / "output_dp"
BACKUP_DIR = BASE_DIR / "backup"
WORK_DIR = BASE_DIR / "patch_work"

BACKUP_BOOT_DIR = BASE_DIR / "backup_boot"
WORKING_BOOT_DIR = BASE_DIR / "working_boot"

INPUT_CURRENT_DIR = BASE_DIR / "input_current"
INPUT_NEW_DIR = BASE_DIR / "input_new"
OUTPUT_ANTI_ROLLBACK_DIR = BASE_DIR / "output_anti_rollback"

IMAGE_DIR = BASE_DIR / "image"
WORKING_DIR = BASE_DIR / "working"
OUTPUT_XML_DIR = BASE_DIR / "output_xml"

PYTHON_EXE = PYTHON_DIR / "python.exe"
ADB_EXE = DOWNLOAD_DIR / "adb.exe"
FASTBOOT_EXE = DOWNLOAD_DIR / "fastboot.exe"
AVBTOOL_PY = DOWNLOAD_DIR / "avbtool.py"
QSAHARASERVER_EXE = TOOLS_DIR / "Qsaharaserver.exe"
FH_LOADER_EXE = TOOLS_DIR / "fh_loader.exe"

def _build_key_map() -> dict[str, Path]:
    if not _config:
        load_config()
    try:
        cfg_map = _config["key_map"]
        return {key: DOWNLOAD_DIR / filename for key, filename in cfg_map.items()}
    except KeyError:
         raise RuntimeError(f"[!] Critical Error: Missing configuration section: [key_map]")

def _init_lazy_constants():
    if _config:
        return
    load_config()
    
    g = globals()
    
    g["MAGISKBOOT_REPO_URL"] = _get_cfg("magiskboot", "repo_url")
    g["MAGISKBOOT_TAG"] = _get_cfg("magiskboot", "tag")

    g["KSU_APK_REPO"] = _get_cfg("kernelsu", "apk_repo")
    g["KSU_APK_TAG"] = _get_cfg("kernelsu", "apk_tag")
    g["RELEASE_OWNER"] = _get_cfg("kernelsu", "release_owner")
    g["RELEASE_REPO"] = _get_cfg("kernelsu", "release_repo")
    g["RELEASE_TAG"] = _get_cfg("kernelsu", "release_tag")
    g["REPO_URL"] = f"https://github.com/{g['RELEASE_OWNER']}/{g['RELEASE_REPO']}"
    g["ANYKERNEL_ZIP_FILENAME"] = _get_cfg("kernelsu", "anykernel_zip")

    g["EDL_LOADER_FILENAME"] = _get_cfg("edl", "loader_filename")
    g["EDL_LOADER_FILE"] = IMAGE_DIR / g["EDL_LOADER_FILENAME"] 

    g["FETCH_VERSION"] = _get_cfg("tools", "fetch_version")
    g["FETCH_REPO_URL"] = _get_cfg("tools", "fetch_repo_url")
    g["PLATFORM_TOOLS_ZIP_URL"] = _get_cfg("tools", "platform_tools_url")
    g["AVB_ARCHIVE_URL"] = _get_cfg("tools", "avb_archive_url")

    g["ROW_PATTERN_DOT"] = bytes.fromhex(_get_cfg("patterns", "row_dot"))
    g["PRC_PATTERN_DOT"] = bytes.fromhex(_get_cfg("patterns", "prc_dot"))
    g["ROW_PATTERN_I"] = bytes.fromhex(_get_cfg("patterns", "row_i"))
    g["PRC_PATTERN_I"] = bytes.fromhex(_get_cfg("patterns", "prc_i"))

    g["KEY_MAP"] = _build_key_map()

    g["COUNTRY_CODES"] = _config.get("country_codes", {})
    g["SORTED_COUNTRY_CODES"] = sorted(g["COUNTRY_CODES"].items(), key=lambda item: item[1])

def __getattr__(name):
    lazy_vars = {
        "MAGISKBOOT_REPO_URL", "MAGISKBOOT_TAG",
        "KSU_APK_REPO", "KSU_APK_TAG", "RELEASE_OWNER", "RELEASE_REPO", "RELEASE_TAG", "REPO_URL", "ANYKERNEL_ZIP_FILENAME",
        "EDL_LOADER_FILENAME", "EDL_LOADER_FILE",
        "FETCH_VERSION", "FETCH_REPO_URL", "PLATFORM_TOOLS_ZIP_URL", "AVB_ARCHIVE_URL",
        "ROW_PATTERN_DOT", "PRC_PATTERN_DOT", "ROW_PATTERN_I", "PRC_PATTERN_I",
        "KEY_MAP", "COUNTRY_CODES", "SORTED_COUNTRY_CODES"
    }
    
    if name in lazy_vars:
        _init_lazy_constants()
        return globals()[name]
    
    raise AttributeError(f"module '{__name__}' has no attribute '{name}'")