@echo off
setlocal

:: This script is called by other batch files.
:: No need for CHCP or initial echo as the caller handles it.

:: ======================================================
:: Variable Definitions (relative to this script's location)
:: ======================================================
set "TOOLS_DIR=%~dp0"
set "BASE_DIR=%~dp0..\"
set "PYTHON_DIR=%BASE_DIR%python3"
set "PYTHON_VERSION=3.14.0"
set "PYTHON_ZIP_URL=https://www.python.org/ftp/python/%PYTHON_VERSION%/python-%PYTHON_VERSION%-embed-amd64.zip"
set "PYTHON_ZIP_PATH=%BASE_DIR%python_embed.zip"
set "PYTHON_PTH_FILE_SRC=%TOOLS_DIR%python314._pth"
set "PYTHON_PTH_FILE_DST=%PYTHON_DIR%\python314._pth"
set "GETPIP_URL=https://bootstrap.pypa.io/get-pip.py"
set "GETPIP_PATH=%PYTHON_DIR%\get-pip.py"
set "FETCH_VERSION=v0.4.6"
set "FETCH_URL=https://github.com/gruntwork-io/fetch/releases/download/%FETCH_VERSION%/fetch_windows_amd64.exe"
set "FETCH_EXE=%TOOLS_DIR%fetch.exe"
set "AVB_DIR=%TOOLS_DIR%avb"
set "AVB_TOOL_PATH=%AVB_DIR%\avbtool.py"
set "AVB_ARCHIVE_URL=https://android.googlesource.com/platform/external/avb/+archive/refs/heads/main.tar.gz"
set "TEMP_ARCHIVE=%TOOLS_DIR%avb_main.tar.gz"

:: ======================================================
:: Create Directories
:: ======================================================
if not exist "%TOOLS_DIR%" mkdir "%TOOLS_DIR%"
if not exist "%AVB_DIR%" mkdir "%AVB_DIR%"

:: ======================================================
:: Check and Install Dependencies
:: ======================================================

:: Check Python
if not exist "%PYTHON_DIR%\python.exe" (
    echo [*] Python not found. Downloading...
    curl -L "%PYTHON_ZIP_URL%" -o "%PYTHON_ZIP_PATH%" || exit /b 1
    echo [*] Extracting Python...
    powershell -Command "Expand-Archive -Path '%PYTHON_ZIP_PATH%' -DestinationPath '%PYTHON_DIR%' -Force"
    del "%PYTHON_ZIP_PATH%"
    if exist "%PYTHON_PTH_FILE_SRC%" copy "%PYTHON_PTH_FILE_SRC%" "%PYTHON_PTH_FILE_DST%"
)

:: Check pip
if not exist "%PYTHON_DIR%\Scripts\pip.exe" (
    echo [*] pip not found. Installing...
    curl -L "%GETPIP_URL%" -o "%GETPIP_PATH%" || exit /b 1
    "%PYTHON_DIR%\python.exe" "%GETPIP_PATH%"
    del "%GETPIP_PATH%"
)

:: Check Python modules
"%PYTHON_DIR%\python.exe" -c "import requests" 2>nul || (
    echo [*] 'requests' module not found. Installing...
    "%PYTHON_DIR%\Scripts\pip.exe" install requests
)
"%PYTHON_DIR%\python.exe" -c "import cryptography" 2>nul || (
    echo [*] 'cryptography' module not found. Installing...
    "%PYTHON_DIR%\Scripts\pip.exe" install cryptography
)

:: Check other tools
if not exist "%FETCH_EXE%" (
    echo [*] fetch.exe not found. Downloading...
    curl -L "%FETCH_URL%" -o "%FETCH_EXE%" || exit /b 1
)

if not exist "%AVB_TOOL_PATH%" (
    echo [*] avbtool not found. Downloading from AOSP...
    curl -L "%AVB_ARCHIVE_URL%" -o "%TEMP_ARCHIVE%" || exit /b 1
    echo [*] Extracting avbtool and keys...
    tar -xzf "%TEMP_ARCHIVE%" -C "%AVB_DIR%" avbtool.py || exit /b 1
    tar -xzf "%TEMP_ARCHIVE%" -C "%AVB_DIR%" --strip-components=2 test/data/testkey_rsa2048.pem || exit /b 1
    tar -xzf "%TEMP_ARCHIVE%" -C "%AVB_DIR%" --strip-components=2 test/data/testkey_rsa4096.pem || exit /b 1
    del "%TEMP_ARCHIVE%"
)

endlocal
exit /b 0