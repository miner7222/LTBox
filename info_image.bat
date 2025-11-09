@echo off
chcp 65001 > nul
setlocal

set "PYTHON_EXE=%~dp0python3\python.exe"
set "MAIN_PY=%~dp0ltbox\main.py"

if not exist "%PYTHON_EXE%" (
    echo [!] Python not found at: %PYTHON_EXE%
    echo [!] Please run install.bat or start.bat first.
    pause
    goto :eof
)
if not exist "%MAIN_PY%" (
    echo [!] Main script not found at: %MAIN_PY%
    pause
    goto :eof
)

if "%~1"=="" (
    echo [!] No files or folders were dragged onto the script.
    echo [!] Please drag and drop .img files or folders containing them.
    pause
    goto :eof
)

echo --- Starting Image Info Scan... ---
"%PYTHON_EXE%" "%MAIN_PY%" info %*

echo.
echo --- Process Complete ---
echo A log file (image_info_....txt) has been created in the main folder.
pause
endlocal