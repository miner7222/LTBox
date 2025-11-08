from pathlib import Path

# --- Base Directories ---
BASE_DIR = Path(__file__).parent.parent.resolve()
LTBOX_DIR = BASE_DIR / "ltbox"
TOOLS_DIR = BASE_DIR / "tools"
PYTHON_DIR = BASE_DIR / "python3"
AVB_DIR = TOOLS_DIR / "avb"
PLATFORM_TOOLS_DIR = TOOLS_DIR / "platform-tools"

# --- Output Directories ---
OUTPUT_DIR = BASE_DIR / "output"
OUTPUT_ROOT_DIR = BASE_DIR / "output_root"
OUTPUT_DP_DIR = BASE_DIR / "output_dp"
BACKUP_DIR = BASE_DIR / "backup"
WORK_DIR = BASE_DIR / "patch_work"

# --- New/Modified Root/Unroot Dirs ---
BACKUP_BOOT_DIR = BASE_DIR / "backup_boot"
WORKING_BOOT_DIR = BASE_DIR / "working_boot"

# --- Input Directories ---
INPUT_CURRENT_DIR = BASE_DIR / "input_current"
INPUT_NEW_DIR = BASE_DIR / "input_new"
OUTPUT_ANTI_ROLLBACK_DIR = BASE_DIR / "output_anti_rollback"

# --- Main Firmware Source / XML Dirs ---
IMAGE_DIR = BASE_DIR / "image"
WORKING_DIR = BASE_DIR / "working"
OUTPUT_XML_DIR = BASE_DIR / "output_xml"

# --- Executable/Script Paths ---
PYTHON_EXE = PYTHON_DIR / "python.exe"
ADB_EXE = PLATFORM_TOOLS_DIR / "adb.exe"
AVBTOOL_PY = AVB_DIR / "avbtool.py"
EDIT_IMAGES_PY = LTBOX_DIR / "edit_images.py"
GET_KERNEL_VER_PY = LTBOX_DIR / "get_kernel_ver.py"
DECRYPT_PY = LTBOX_DIR / "decrypt_x.py"
QSAHARASERVER_EXE = TOOLS_DIR / "Qsaharaserver.exe"
FH_LOADER_EXE = TOOLS_DIR / "fh_loader.exe"

# --- MagiskBoot ---
MAGISKBOOT_REPO_URL = "https://github.com/PinNaCode/magiskboot_build"
MAGISKBOOT_TAG = "last-ci"

# --- KernelSU ---
KSU_APK_REPO = "KernelSU-Next/KernelSU-Next"
KSU_APK_TAG = "v1.1.1"
RELEASE_OWNER = "WildKernels"
RELEASE_REPO = "GKI_KernelSU_SUSFS"
RELEASE_TAG = "v1.5.12-r16"
REPO_URL = f"https://github.com/{RELEASE_OWNER}/{RELEASE_REPO}"
ANYKERNEL_ZIP_FILENAME = "AnyKernel3.zip"

# --- EDL ---
EDL_NG_REPO_URL = "https://github.com/strongtz/edl-ng"
EDL_NG_TAG = "v1.4.1"
EDL_LOADER_FILENAME = "xbl_s_devprg_ns.melf"
EDL_LOADER_FILE = IMAGE_DIR / EDL_LOADER_FILENAME 
EDL_LOADER_FILE_IMAGE = IMAGE_DIR / EDL_LOADER_FILENAME

# --- AVB Keys ---
KEY_MAP = {
    "2597c218aae470a130f61162feaae70afd97f011": AVB_DIR / "testkey_rsa4096.pem",
    "cdbb77177f731920bbe0a0f94f84d9038ae0617d": AVB_DIR / "testkey_rsa2048.pem"
}

# --- Country Codes for devinfo/persist patching ---
COUNTRY_CODES = {
    "AE": "United Arab Emirates", "AM": "Armenia", "AR": "Argentina", "AT": "Austria", "AU": "Australia",
    "AZ": "Azerbaijan", "BE": "Belgium", "BG": "Bulgaria", "BH": "Bahrain", "BR": "Brazil",
    "CA": "Canada", "CH": "Switzerland", "CL": "Chile", "CN": "China", "CO": "Colombia", "CR": "Costa Rica",
    "CY": "Cyprus", "CZ": "Czech Republic", "DE": "Germany", "DK": "Denmark", "EC": "Ecuador",
    "EE": "Estonia", "EG": "Egypt", "ES": "Spain", "FI": "Finland", "FR": "France",
    "GB": "United Kingdom", "GE": "Georgia", "GH": "Ghana", "GR": "Greece", "GT": "Guatemala",
    "HK": "Hong Kong", "HR": "Croatia", "HU": "Hungary", "ID": "Indonesia", "IL": "Israel",
    "IN": "India", "IS": "Iceland", "IT": "Italy", "JO": "Jordan", "JP": "Japan",
    "KE": "Kenya", "KG": "Kyrgyzstan", "KR": "Korea", "KW": "Kuwait", "KZ": "Kazakhstan",
    "LB": "Lebanon", "LT": "Lithuania", "LV": "Latvia", "MA": "Morocco", "MD": "Moldova",
    "MX": "Mexico", "MY": "Malaysia", "MZ": "Mozambique", "NG": "Nigeria", "NL": "Netherlands",
    "NO": "Norway", "NZ": "New Zealand", "OM": "Oman", "PA": "Panama", "PE": "Peru",
    "PH": "Philippines", "PK": "Pakistan", "PL": "Poland", "PT": "Portugal", "QA": "Qatar",
    "RO": "Romania", "RS": "Serbia", "RU": "Russia", "SA": "Saudi Arabia", "SE": "Sweden",
    "SG": "Singapore", "SI": "Slovenia", "SK": "Slovakia", "SV": "El Salvador", "TH": "Thailand",
    "TJ": "Tajikistan", "TN": "Tunisia", "TR": "Turkey", "TW": "Taiwan", "TZ": "Tanzania",
    "UA": "Ukraine", "UG": "Uganda", "US": "United States of America", "UY": "Uruguay",
    "UZ": "Uzbekistan", "VE": "Venezuela", "VN": "Vietnam", "ZA": "South Africa"
}