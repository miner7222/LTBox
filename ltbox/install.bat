@echo off
setlocal

:: This script is called by other batch files.
:: ======================================================
:: Variable Definitions (relative to this script's location)
:: ======================================================
set "LTBOX_DIR=%~dp0"
set "BASE_DIR=%~dp0..\"
set "TOOLS_DIR=%BASE_DIR%tools\"
set "PYTHON_DIR=%BASE_DIR%python3"
set "PYTHON_VERSION=3.14.0"
set "PYTHON_ZIP_URL=https://www.python.org/ftp/python/%PYTHON_VERSION%/python-%PYTHON_VERSION%-embed-amd64.zip"
set "PYTHON_ZIP_PATH=%BASE_DIR%python_embed.zip"
set "PYTHON_PTH_FILE_SRC=%LTBOX_DIR%python314._pth"
set "PYTHON_PTH_FILE_DST=%PYTHON_DIR%\python314._pth"
set "GETPIP_URL=https://bootstrap.pypa.io/get-pip.py"
set "GETPIP_PATH=%PYTHON_DIR%\get-pip.py"

:: ======================================================
:: Create Directories
:: ======================================================
if not exist "%TOOLS_DIR%" mkdir "%TOOLS_DIR%"

:: ======================================================
:: Check and Install Dependencies
:: ======================================================

:: Check Python
if not exist "%PYTHON_DIR%\python.exe" (
    echo [*] Python not found. Downloading...
    curl -L "%PYTHON_ZIP_URL%" -o "%PYTHON_ZIP_PATH%" || exit /b 1
    echo [*] Extracting Python...
    mkdir "%PYTHON_DIR%"
    tar -xf "%PYTHON_ZIP_PATH%" -C "%PYTHON_DIR%"
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
echo [*] Installing required Python modules from requirements.txt...
"%PYTHON_DIR%\Scripts\pip.exe" install -r "%BASE_DIR%requirements.txt"

:: Download all other tools using the downloader script
echo [*] Installing external tools (fetch, adb, avbtool)...
"%PYTHON_DIR%\python.exe" "%LTBOX_DIR%downloader.py" install_base_tools

endlocal
exit /b 0