@echo off
setlocal
set "PYTHON_EXE=%~dp0python3\python.exe"
set "MAIN_SCRIPT=%~dp0main.py"

if not exist "%PYTHON_EXE%" (
    echo [!] Python executable not found. Please run install.bat first.
    pause
    exit /b
)

"%PYTHON_EXE%" "%MAIN_SCRIPT%" info %*