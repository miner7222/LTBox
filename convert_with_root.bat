@echo off
setlocal enabledelayedexpansion

set "TARGET_KERNEL_VERSION=6.1.112"
set "ANYKERNEL_URL=https://github.com/WildKernels/GKI_KernelSU_SUSFS/releases/download/v1.5.9-r36/WKSU-13861-android14-6.1.112-2024-11-AnyKernel3.zip"
set "ANYKERNEL_ZIP=AnyKernel3.zip"

set "TOOLS_DIR=%~dp0tools\"
set "CURRENT_DIR=%cd%\"
set "PYTHON=%~dp0python3\python.exe"

echo --- Checking for required tools ---
echo.
if not exist "%TOOLS_DIR%" mkdir "%TOOLS_DIR%"

echo [*] Checking for Python in local 'python3' folder...
if not exist "%PYTHON%" (
    echo [!] '%PYTHON%' was not found.
    echo Please ensure a portable Python installation exists at that location.
    pause
    exit /b
)
echo [+] Python found at: %PYTHON%
echo.
echo [*] Checking for magiskboot.exe...
if exist "%TOOLS_DIR%magiskboot.exe" (
    echo [+] magiskboot.exe is present.
) else (
    echo [!] 'magiskboot.exe' not found. Attempting to download...
    powershell -Command "Invoke-WebRequest -Uri 'https://github.com/CYRUS-STUDIO/MagiskBootWindows/raw/refs/heads/main/magiskboot.exe' -OutFile '%TOOLS_DIR%magiskboot.exe'"
    if exist "%TOOLS_DIR%magiskboot.exe" (echo [+] Download successful.) else (echo [!] Download failed. Aborting. & pause & exit /b)
)
echo.
set "PATH=%TOOLS_DIR%;%PATH%"

if not exist "boot.img" (
    echo [!] 'boot.img' not found in the current directory.
    echo Please place the script in the same folder as boot.img.
    pause
    exit /b
)

echo --- Backing up original boot.img ---
copy "%CURRENT_DIR%boot.img" "%CURRENT_DIR%boot.bak.img"

echo --- Starting boot.img patching process ---
echo.
set "WORK_DIR=%~dp0patch_work"
if exist "%WORK_DIR%" rd /s /q "%WORK_DIR%"
mkdir "%WORK_DIR%"
cd "%WORK_DIR%"

copy "%CURRENT_DIR%boot.img" . >nul

echo [1/6] Unpacking boot image...
magiskboot.exe unpack boot.img
if not exist "kernel" (
    echo [!] Failed to unpack boot.img. The image might be invalid.
    goto cleanup_and_abort
)
echo [+] Unpack successful.
echo.

echo [2/6] Verifying kernel version...
set "EXTRACTED_VERSION_STRING="
"%PYTHON%" "%TOOLS_DIR%get_kernel_ver.py" kernel > version.tmp 2>nul
if exist version.tmp (
    set /p EXTRACTED_VERSION_STRING=<version.tmp
    del version.tmp
)

if not defined EXTRACTED_VERSION_STRING (
    echo [!] Could not read kernel version string from the 'kernel' file.
    goto cleanup_and_abort
)

echo [+] Found version string: !EXTRACTED_VERSION_STRING!
echo !EXTRACTED_VERSION_STRING! | findstr /b "%TARGET_KERNEL_VERSION%" >nul
if !errorlevel! neq 0 (
    echo [!] ERROR: Kernel version is NOT %TARGET_KERNEL_VERSION%.
    echo Script will now abort to prevent damage.
    goto cleanup_and_abort
)
echo [+] Kernel version matches.
echo.

echo [3/6] Downloading GKI Kernel...
powershell -Command "Invoke-WebRequest -Uri '%ANYKERNEL_URL%' -OutFile '%ANYKERNEL_ZIP%'"
if not exist "%ANYKERNEL_ZIP%" (
    echo [!] Failed to download the kernel zip file.
    goto cleanup_and_abort
)
echo [+] Download complete.
echo.

echo [4/6] Extracting new kernel image...
mkdir extracted_kernel
powershell -Command "Expand-Archive -Path '%ANYKERNEL_ZIP%' -DestinationPath 'extracted_kernel' -Force"
if not exist "extracted_kernel\Image" (
    echo [!] 'Image' file not found in the downloaded zip.
    goto cleanup_and_abort
)
echo [+] Extraction successful.
echo.

echo [5/6] Replacing original kernel with the new one...
move /y "extracted_kernel\Image" "kernel" >nul
echo [+] Kernel replaced.
echo.
echo [6/6] Repacking boot image...
magiskboot.exe repack boot.img
if not exist "new-boot.img" (
    echo [!] Failed to repack the boot image.
    goto cleanup_and_abort
)
echo [+] Repack successful.
echo.

echo --- Finalizing ---
move "new-boot.img" "%CURRENT_DIR%boot.root.img"
cd "%CURRENT_DIR%"
rd /s /q "%WORK_DIR%"

echo --- Cleaning up ---
del boot.img
if exist "%WORK_DIR%" rd /s /q "%WORK_DIR%"

echo.
echo =============================================================
echo  SUCCESS!
echo  Patched image has been saved as:
echo  %CURRENT_DIR%boot.root.img
echo =============================================================
echo.
echo --- Handing over to convert.bat ---
echo.
call convert.bat with_boot