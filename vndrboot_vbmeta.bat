@echo off
chcp 65001 > nul
setlocal

echo --- Initializing... ---
call "%~dp0tools\install.bat"
if errorlevel 1 (
    echo [!] Dependency installation failed. Aborting.
    pause
    exit /b 1
)
echo.

set "PYTHON_EXE=%~dp0python3\python.exe"
set "MAIN_PY=%~dp0main.py"

echo --- Converting vendor_boot and Remaking vbmeta ---
echo.

"%PYTHON_EXE%" "%MAIN_PY%" convert

endlocal
pause