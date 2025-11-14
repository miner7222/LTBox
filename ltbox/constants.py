import json
import sys
from pathlib import Path

BASE_DIR = Path(__file__).parent.parent.resolve()
LTBOX_DIR = BASE_DIR / "ltbox"
TOOLS_DIR = BASE_DIR / "tools"
DOWNLOAD_DIR = TOOLS_DIR / "dl"
PYTHON_DIR = BASE_DIR / "python3"

CONFIG_FILE = LTBOX_DIR / "config.json"
_config = {}

def load_config():
    global _config
    if CONFIG_FILE.exists():
        try:
            with open(CONFIG_FILE, 'r', encoding='utf-8') as f:
                _config = json.load(f)
        except Exception as e:
            print(f"[!] Critical Error: Failed to load config.json: {e}", file=sys.stderr)
            sys.exit(1)
    else:
        print(f"[!] Critical Error: Configuration file missing: {CONFIG_FILE}", file=sys.stderr)
        sys.exit(1)

def _get_cfg(section: str, key: str) -> str:
    if not _config:
        load_config()
    try:
        return _config[section][key]
    except KeyError:
        print(f"[!] Critical Error: Missing configuration key: [{section}][{key}]", file=sys.stderr)
        sys.exit(1)

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

ROW_PATTERN_DOT = b"\x2E\x52\x4F\x57"
PRC_PATTERN_DOT = b"\x2E\x50\x52\x43"
ROW_PATTERN_I = b"\x49\x52\x4F\x57"
PRC_PATTERN_I = b"\x49\x50\x52\x43"

KEY_MAP = {
    "2597c218aae470a130f61162feaae70afd97f011": DOWNLOAD_DIR / "testkey_rsa4096.pem",
    "cdbb77177f731920bbe0a0f94f84d9038ae0617d": DOWNLOAD_DIR / "testkey_rsa2048.pem"
}

COUNTRY_CODES = _config.get("country_codes", {})