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

echo --- Patching boot.img for Root ---
echo.

"%PYTHON_EXE%" "%MAIN_PY%" root

endlocal
pause