@echo off
chcp 65001 > nul
setlocal

:: --- 1. Initialization and Dependency Check ---
echo --- Initializing LTBox... ---
call "%~dp0ltbox\install.bat"
if errorlevel 1 (
    echo [!] Dependency installation failed. Please check ltbox\install.bat.
    pause
    goto :eof
)

:: --- 2. Set Python and Main Script Paths ---
set "PYTHON_EXE=%~dp0python3\python.exe"
set "MAIN_PY=%~dp0ltbox\main.py"

if not exist "%PYTHON_EXE%" (
    echo [!] Python not found at: %PYTHON_EXE%
    echo [!] Please run ltbox\install.bat first.
    pause
    goto :eof
)
if not exist "%MAIN_PY%" (
    echo [!] Main script not found at: %MAIN_PY%
   
    pause
    goto :eof
)

:: --- 3. Main Menu Loop ---
:main_menu
cls
echo.
echo   ==========================================================
echo     LTBox - Main
echo   ==========================================================
echo.
echo     1. Install ROW ROM to PRC device (WIPE DATA)
echo     2. Update ROW ROM on PRC device (NO WIPE)
echo     3. Create Rooted boot.img
echo.
echo     a. Advanced
echo     x. Exit
echo.
echo   ==========================================================
echo.
set "CHOICE="
set /p "CHOICE=    Enter your choice (1, 2, 3, a, x): "

if /I "%CHOICE%"=="1" call :run_task patch_all_wipe "Install ROW ROM (WIPE DATA)"
if /I "%CHOICE%"=="2" call :run_task patch_all "Update ROW ROM (NO WIPE)"
if /I "%CHOICE%"=="3" call :run_task root "Root boot.img"
if /I "%CHOICE%"=="a" goto :advanced_menu
if /I "%CHOICE%"=="x" goto :cleanup

:: Handle invalid input
echo.
echo     [!] Invalid choice. Please enter 1, 2, 3, a, or x.
pause
goto :main_menu


:: --- 4. Advanced Menu ---
:advanced_menu
cls
echo.
echo   ==========================================================
echo     LTBox - Advanced
echo   ==========================================================
echo.
echo     1. Convert PRC to ROW in ROM
echo     2. Dump devinfo/persist from device
echo     3. Patch devinfo/persist to reset region code
echo     4. Write devinfo/persist to device
echo     5. Detect Anti-Rollback from device
echo     6. Patch rollback indices in ROM
echo     7. Write Anti-Anti-Rollback to device
echo     8. Convert x files to xml (WIPE DATA)
echo     9. Convert x files to xml & Modify (NO WIPE)
echo     10. Flash ROM to device
echo.
echo     11. Clean workspace
echo     m. Back to Main
echo.
echo   ==========================================================
echo.
set "ADV_CHOICE="
set /p "ADV_CHOICE=    Enter your choice (1-11, m): "

if "%ADV_CHOICE%"=="1" call :run_task convert "Convert PRC to ROW in ROM"
if "%ADV_CHOICE%"=="2" call :run_task read_edl "Dump devinfo/persist from device"
if "%ADV_CHOICE%"=="3" call :run_task edit_dp "Patch devinfo/persist to reset region code"
if "%ADV_CHOICE%"=="4" call :run_task write_edl "Write devinfo/persist to device"
if "%ADV_CHOICE%"=="5" call :run_task read_anti_rollback "Detect Anti-Rollback from device"
if "%ADV_CHOICE%"=="6" call :run_task patch_anti_rollback "Patch rollback indices in ROM"
if "%ADV_CHOICE%"=="7" call :run_task write_anti_rollback "Write Anti-Anti-Rollback to device"
if "%ADV_CHOICE%"=="8" call :run_task modify_xml_wipe "Convert x files to xml (WIPE DATA)"
if "%ADV_CHOICE%"=="9" call :run_task modify_xml "Convert & Modify x files to xml (NO WIPE)"
if "%ADV_CHOICE%"=="10" call :run_task flash_edl "Flash ROM to device"

if "%ADV_CHOICE%"=="11" (
    cls
  
    echo ==========================================================
    echo  Starting Task: [Workspace Cleanup]...
    echo ==========================================================
    echo.
    "%PYTHON_EXE%" "%MAIN_PY%" clean
    echo.
    echo ==========================================================
    echo  Task [Workspace Cleanup] has completed.
    echo ==========================================================
    echo.
    echo Press any key to exit...
    pause > nul
    goto :cleanup
)

if /I "%ADV_CHOICE%"=="m" goto :main_menu

echo.
echo     [!] Invalid choice. Please enter a number from 1-11, or m.
pause
goto :advanced_menu


:: --- 5. Task Execution Subroutine ---
:run_task
cls
echo ==========================================================
echo  Starting Task: [%~2]...
echo ==========================================================
echo.
:: %1 is the main.py argument (e.g., convert), %~2 is the description string
"%PYTHON_EXE%" "%MAIN_PY%" %1

echo.
echo ==========================================================
echo  Task [%~2] has completed.
echo ==========================================================
echo.
echo Press any key to return...
pause > nul

:: If the task was from the Main menu, return to Main
if "%1"=="patch_all_wipe" goto :main_menu
if "%1"=="patch_all" goto :main_menu
if "%1"=="root" goto :main_menu

:: Stay in advanced menu for advanced tasks
goto :advanced_menu


:: --- 6. Exit ---
:cleanup
endlocal
echo.
echo Exiting LTBox.
goto :eof