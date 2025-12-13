import json
import sys
from pathlib import Path
from typing import Any, Dict, List, Tuple

class LTBoxConfig:
    def __init__(self):
        self._loaded = False
        self._config_data: Dict[str, Any] = {}
        
        # --- Base Paths ---
        self.base_dir = Path(__file__).parent.parent.parent.resolve()
        self.ltbox_dir = self.base_dir / "bin" / "ltbox"
        self.tools_dir = self.base_dir / "bin" / "tools"
        self.download_dir = self.tools_dir / "dl"
        self.python_dir = self.base_dir / "bin" / "python3"
        self.config_file = self.ltbox_dir / "config.json"

        # --- Output & Work Paths ---
        self.output_dir = self.base_dir / "output"
        self.output_root_dir = self.base_dir / "output_root"
        self.output_root_lkm_dir = self.base_dir / "output_root_lkm"
        self.output_dp_dir = self.base_dir / "output_dp"
        self.backup_dir = self.base_dir / "backup"
        self.work_dir = self.base_dir / "patch_work"
        
        self.backup_boot_dir = self.base_dir / "backup_boot"
        self.backup_init_boot_dir = self.base_dir / "backup_init_boot"
        self.working_boot_dir = self.base_dir / "working_boot"
        
        self.output_anti_rollback_dir = self.base_dir / "output_anti_rollback"
        
        self.image_dir = self.base_dir / "image"
        self.working_dir = self.base_dir / "working"
        self.output_xml_dir = self.base_dir / "output_xml"

        # --- File Name Constants ---
        self.fn_boot = "boot.img"
        self.fn_init_boot = "init_boot.img"
        self.fn_vendor_boot = "vendor_boot.img"
        self.fn_vbmeta = "vbmeta.img"
        self.fn_vbmeta_system = "vbmeta_system.img"
        self.fn_devinfo = "devinfo.img"
        self.fn_persist = "persist.img"

        self.fn_boot_bak = "boot.bak.img"
        self.fn_init_boot_bak = "init_boot.bak.img"
        self.fn_vbmeta_bak = "vbmeta.bak.img"
        self.fn_vendor_boot_bak = "vendor_boot.bak.img"

        self.fn_boot_root = "boot.root.img"
        self.fn_init_boot_root = "init_boot.root.img"
        self.fn_vbmeta_root = "vbmeta.root.img"

        self.fn_vendor_boot_prc = "vendor_boot_prc.img"

        # --- Executables ---
        self.python_exe = self.python_dir / "python.exe"
        self.adb_exe = self.download_dir / "adb.exe"
        self.fastboot_exe = self.download_dir / "fastboot.exe"
        self.avbtool_py = self.download_dir / "avbtool.py"
        self.qsaharaserver_exe = self.tools_dir / "Qsaharaserver.exe"
        self.edl_exe = self.tools_dir / "fh_loader.exe"

    def load(self) -> None:
        if self._loaded:
            return

        if self.config_file.exists():
            try:
                with open(self.config_file, 'r', encoding='utf-8') as f:
                    self._config_data = json.load(f)
                self._loaded = True
            except Exception as e:
                raise RuntimeError(f"[!] Critical Error: Failed to load config.json: {e}")
        else:
            raise RuntimeError(f"[!] Critical Error: Configuration file missing: {self.config_file}")

    def _get_val(self, section: str, key: str, default: Any = None) -> Any:
        self.load()
        try:
            return self._config_data[section][key]
        except KeyError:
            if default is not None:
                return default
            raise RuntimeError(f"[!] Critical Error: Missing configuration key: [{section}][{key}]")

    # --- Config Properties ---

    @property
    def magiskboot_repo_url(self) -> str:
        return self._get_val("magiskboot", "repo_url")

    @property
    def magiskboot_tag(self) -> str:
        return self._get_val("magiskboot", "tag")

    @property
    def ksu_apk_repo(self) -> str:
        return self._get_val("kernelsu-next", "apk_repo")

    @property
    def ksu_apk_tag(self) -> str:
        return self._get_val("kernelsu-next", "apk_tag")

    @property
    def sukisu_repo(self) -> str:
        return self._get_val("sukisu-ultra", "repo")

    @property
    def sukisu_workflow(self) -> str:
        return self._get_val("sukisu-ultra", "workflow")

    @property
    def release_owner(self) -> str:
        return self._get_val("kernelsu-next", "release_owner")

    @property
    def release_repo(self) -> str:
        return self._get_val("kernelsu-next", "release_repo")

    @property
    def release_tag(self) -> str:
        return self._get_val("kernelsu-next", "release_tag")

    @property
    def repo_url(self) -> str:
        return f"https://github.com/{self.release_owner}/{self.release_repo}"

    @property
    def anykernel_zip_filename(self) -> str:
        return self._get_val("kernelsu-next", "anykernel_zip")

    @property
    def sukisu_manager_url(self) -> str:
        return f"https://nightly.link/{self.sukisu_repo}/actions/runs/{self.sukisu_workflow}/Spoofed-Manager.zip"

    def get_sukisu_lkm_url(self, kernel_mapped_name: str) -> str:
        return f"https://nightly.link/{self.sukisu_repo}/actions/runs/{self.sukisu_workflow}/{identifier}-lkm.zip"

    @property
    def sukisu_ksuinit_url(self) -> str:
        return f"https://nightly.link/{self.sukisu_repo}/actions/runs/{self.sukisu_workflow}/ksuinit-aarch64-linux-android.zip"

    @property
    def kernel_android_map(self) -> Dict[str, str]:
        return {
            "5.10": "android13",
            "5.15": "android14",
            "6.1": "android14",
            "6.6": "android15",
            "6.12": "android16"
        }

    @property
    def edl_loader_filename(self) -> str:
        return self._get_val("edl", "loader_filename")

    @property
    def edl_loader_file(self) -> Path:
        return self.image_dir / self.edl_loader_filename

    @property
    def platform_tools_zip_url(self) -> str:
        return self._get_val("tools", "platform_tools_url")

    @property
    def avb_archive_url(self) -> str:
        return self._get_val("tools", "avb_archive_url")

    @property
    def row_pattern_dot(self) -> bytes:
        return bytes.fromhex(self._get_val("patterns", "row_dot"))

    @property
    def prc_pattern_dot(self) -> bytes:
        return bytes.fromhex(self._get_val("patterns", "prc_dot"))

    @property
    def row_pattern_i(self) -> bytes:
        return bytes.fromhex(self._get_val("patterns", "row_i"))

    @property
    def prc_pattern_i(self) -> bytes:
        return bytes.fromhex(self._get_val("patterns", "prc_i"))

    @property
    def key_map(self) -> Dict[str, Path]:
        self.load()
        try:
            cfg_map = self._config_data.get("key_map", {})
            return {key: self.download_dir / filename for key, filename in cfg_map.items()}
        except KeyError:
             raise RuntimeError("[!] Critical Error: Missing configuration section: [key_map]")

    @property
    def country_codes(self) -> Dict[str, str]:
        self.load()
        return self._config_data.get("country_codes", {})

    @property
    def sorted_country_codes(self) -> List[Tuple[str, str]]:
        return sorted(self.country_codes.items(), key=lambda item: item[1])


# --- Singleton Instance ---
CONF = LTBoxConfig()

# --- Module Level Exports (Backward Compatibility) ---

BASE_DIR = CONF.base_dir
LTBOX_DIR = CONF.ltbox_dir
TOOLS_DIR = CONF.tools_dir
DOWNLOAD_DIR = CONF.download_dir
PYTHON_DIR = CONF.python_dir
CONFIG_FILE = CONF.config_file

OUTPUT_DIR = CONF.output_dir
OUTPUT_ROOT_DIR = CONF.output_root_dir
OUTPUT_ROOT_LKM_DIR = CONF.output_root_lkm_dir
OUTPUT_DP_DIR = CONF.output_dp_dir
BACKUP_DIR = CONF.backup_dir
WORK_DIR = CONF.work_dir
BACKUP_BOOT_DIR = CONF.backup_boot_dir
BACKUP_INIT_BOOT_DIR = CONF.backup_init_boot_dir
WORKING_BOOT_DIR = CONF.working_boot_dir
OUTPUT_ANTI_ROLLBACK_DIR = CONF.output_anti_rollback_dir
IMAGE_DIR = CONF.image_dir
WORKING_DIR = CONF.working_dir
OUTPUT_XML_DIR = CONF.output_xml_dir

FN_BOOT = CONF.fn_boot
FN_INIT_BOOT = CONF.fn_init_boot
FN_VENDOR_BOOT = CONF.fn_vendor_boot
FN_VBMETA = CONF.fn_vbmeta
FN_VBMETA_SYSTEM = CONF.fn_vbmeta_system
FN_DEVINFO = CONF.fn_devinfo
FN_PERSIST = CONF.fn_persist

FN_BOOT_BAK = CONF.fn_boot_bak
FN_INIT_BOOT_BAK = CONF.fn_init_boot_bak
FN_VBMETA_BAK = CONF.fn_vbmeta_bak
FN_VENDOR_BOOT_BAK = CONF.fn_vendor_boot_bak

FN_BOOT_ROOT = CONF.fn_boot_root
FN_INIT_BOOT_ROOT = CONF.fn_init_boot_root
FN_VBMETA_ROOT = CONF.fn_vbmeta_root
FN_VENDOR_BOOT_PRC = CONF.fn_vendor_boot_prc

PYTHON_EXE = CONF.python_exe
ADB_EXE = CONF.adb_exe
FASTBOOT_EXE = CONF.fastboot_exe
AVBTOOL_PY = CONF.avbtool_py
QSAHARASERVER_EXE = CONF.qsaharaserver_exe
EDL_EXE = CONF.edl_exe

try:
    MAGISKBOOT_REPO_URL = CONF.magiskboot_repo_url
    MAGISKBOOT_TAG = CONF.magiskboot_tag
    KSU_APK_REPO = CONF.ksu_apk_repo
    KSU_APK_TAG = CONF.ksu_apk_tag
    SUKISU_REPO = CONF.sukisu_repo
    SUKISU_WORKFLOW = CONF.sukisu_workflow
    KERNEL_ANDROID_MAP = CONF.kernel_android_map
    RELEASE_OWNER = CONF.release_owner
    RELEASE_REPO = CONF.release_repo
    RELEASE_TAG = CONF.release_tag
    REPO_URL = CONF.repo_url
    ANYKERNEL_ZIP_FILENAME = CONF.anykernel_zip_filename

    EDL_LOADER_FILENAME = CONF.edl_loader_filename
    EDL_LOADER_FILE = CONF.edl_loader_file
    
    PLATFORM_TOOLS_ZIP_URL = CONF.platform_tools_zip_url
    AVB_ARCHIVE_URL = CONF.avb_archive_url
    
    ROW_PATTERN_DOT = CONF.row_pattern_dot
    PRC_PATTERN_DOT = CONF.prc_pattern_dot
    ROW_PATTERN_I = CONF.row_pattern_i
    PRC_PATTERN_I = CONF.prc_pattern_i
    
    KEY_MAP = CONF.key_map
    COUNTRY_CODES = CONF.country_codes
    SORTED_COUNTRY_CODES = CONF.sorted_country_codes

except RuntimeError:
    pass