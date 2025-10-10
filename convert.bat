@echo off
setlocal enabledelayedexpansion

if "%~1"=="with_boot" (
    set "PROCESS_BOOT=true"
)

echo --- Checking for required files ---
set "MISSING_FILE="
if not exist "%~dp0python3\python.exe" set "MISSING_FILE=Python Environment"
if not exist "%~dp0key\testkey_rsa4096.pem" set "MISSING_FILE=RSA4096 Key"
if not exist "%~dp0key\testkey_rsa2048.pem" set "MISSING_FILE=RSA2048 Key"
if not exist "%~dp0tools\avbtool.py" set "MISSING_FILE=avbtool"

if defined MISSING_FILE (
    echo [!] Error: Dependency '%MISSING_FILE%' is missing.
    echo Please run 'install.bat' first to download all required files.
    pause
    exit /b
)
echo [+] All dependencies are present.
echo.

set "PYTHON=%~dp0python3\python.exe"
set "PY_AVBTOOL=%~dp0tools\avbtool.py"
set "PY_EDIT=%~dp0tools\edit_vndrboot.py"
set "PY_PARSE=%~dp0tools\parse_info.py"
set "PATH=%~dp0tools;%PATH%"

echo [*] Cleaning up old folders...
if exist output rmdir /s /q output
echo.
echo --- Backing up original images ---
if not exist vendor_boot.img (
    echo [!] 'vendor_boot.img' not found! Aborting.
    pause
    exit /b
)
if not exist vbmeta.img (
    echo [!] 'vbmeta.img' not found! Aborting.
    pause
    exit /b
)

move vendor_boot.img vendor_boot.bak.img
copy vbmeta.img vbmeta.bak.img
echo [+] Backup complete.
echo.

echo --- Starting PRC/ROW Conversion ---
"%PYTHON%" "%PY_EDIT%" vendor_boot.bak.img
if errorlevel 1 (
    echo [!] Python script failed. Aborting.
    pause
    exit /b
)
echo.
echo [*] Verifying conversion result...
if exist vendor_boot_prc.img (
    echo [+] Conversion to PRC successful.
) else (
    echo [!] 'vendor_boot_prc.img' was not created. No changes made.
    pause
    exit /b
)
echo.
echo --- Extracting Image Information ---
"%PYTHON%" "%PY_PARSE%" vendor_boot.bak.img "%PY_AVBTOOL%" vbmeta.bak.img > info.tmp
if errorlevel 1 (
    del info.tmp
    echo [!] Failed to execute 'parse_info.py'. Aborting.
    pause
    exit /b
)

for /f "usebackq tokens=1,* delims==" %%a in (info.tmp) do (
    set "%%a=%%b"
)
del info.tmp

if not defined IMG_SIZE (
    echo [!] Failed to read IMG_SIZE from temp file. Aborting.
    pause
    exit /b
)
if not defined PUBLIC_KEY (
    echo [!] Failed to read PUBLIC_KEY from temp file. Aborting.
    pause
    exit /b
)
echo.

echo --- Adding Hash Footer to vendor_boot ---
set "PROP_VAL_CLEAN=!PROP_VAL:~1,-1!"
<nul (set /p ".=!PROP_VAL_CLEAN!") > prop_val.tmp

"%PYTHON%" "%PY_AVBTOOL%" add_hash_footer ^
    --image vendor_boot_prc.img ^
    --partition_size !IMG_SIZE! ^
    --partition_name vendor_boot ^
    --rollback_index 0 ^
    --salt !SALT! ^
    --prop_from_file "!PROP_KEY!:prop_val.tmp"

del prop_val.tmp

if errorlevel 1 (
    echo [!] Failed to add hash footer to 'vendor_boot_prc.img'. Aborting.
    pause
    exit /b
)
echo.
if defined PROCESS_BOOT (
    echo --- Processing boot image ---
    echo [*] Extracting info from boot.bak.img...
    "%PYTHON%" "%PY_AVBTOOL%" info_image --image boot.bak.img > boot_info.tmp
    if errorlevel 1 (
        del boot_info.tmp
        echo [!] Failed to get info from 'boot.bak.img'. Aborting.
        pause
        exit /b
    )

    set "BOOT_PROPS_FILES="
    set /a prop_count=0
    for /f "usebackq tokens=1,* delims=->" %%a in (`findstr /C:"Prop:" boot_info.tmp`) do (
        set /a prop_count+=1
        set "key_part=%%a"
        set "val_part=%%b"

        rem Clean up key
        for /f "tokens=2" %%k in ("!key_part!") do set "PROP_KEY=%%k"

        rem Clean up value
        set "PROP_VAL=!val_part:~2,-1!"

        rem Write value to temp file
        <nul (set /p ".=!PROP_VAL!") > "prop_boot_!prop_count!.tmp"

        rem Build argument string
        set "BOOT_PROPS_FILES=!BOOT_PROPS_FILES! --prop_from_file "!PROP_KEY!:prop_boot_!prop_count!.tmp""
    )

    for /f "usebackq tokens=3" %%a in (`findstr /C:"Image size:" boot_info.tmp`) do ( set "BOOT_IMG_SIZE=%%a" )
    for /f "usebackq tokens=3,*" %%a in (`findstr /C:"Partition Name:" boot_info.tmp`) do ( set "BOOT_PART_NAME=%%a" )
    for /f "usebackq tokens=2,*" %%a in (`findstr /C:"Salt:" boot_info.tmp`) do ( set "BOOT_SALT=%%a" )
    for /f "usebackq tokens=3,*" %%a in (`findstr /C:"Rollback Index:" boot_info.tmp`) do ( set "BOOT_ROLLBACK_INDEX=%%a" )
    del boot_info.tmp

    if not defined BOOT_IMG_SIZE (
        echo [!] Failed to read BOOT_IMG_SIZE from 'boot.bak.img' info. Aborting.
        pause
        exit /b
    )

	echo [*] Verifying vbmeta key for boot image...
	if "!PUBLIC_KEY!"=="2597c218aae470a130f61162feaae70afd97f011" (
		echo [+] Matched RSA4096 key.
		set "KEY_FILE=key\testkey_rsa4096.pem"
	) else if "!PUBLIC_KEY!"=="cdbb77177f731920bbe0a0f94f84d9038ae0617d" (
		echo [+] Matched RSA2048 key.
		set "KEY_FILE=key\testkey_rsa2048.pem"
	) else (
		echo [!] Public key '!PUBLIC_KEY!' did not match known keys. Aborting.
		pause
		exit /b
	)
	if not defined ALGORITHM (
		echo [!] Could not determine Algorithm from 'vbmeta.img'. Aborting.
		pause
		exit /b
	)
	echo.
    echo [*] Adding new hash footer to 'boot.root.img'...
    "%PYTHON%" "%PY_AVBTOOL%" add_hash_footer ^
        --image boot.root.img ^
		--key "%~dp0!KEY_FILE!" ^
		--algorithm !ALGORITHM! ^
        --partition_size !BOOT_IMG_SIZE! ^
        --partition_name !BOOT_PART_NAME! ^
        --rollback_index !BOOT_ROLLBACK_INDEX! ^
        --salt !BOOT_SALT! ^
        !BOOT_PROPS_FILES!
    if errorlevel 1 (
        echo [!] Failed to add hash footer to 'boot.root.img'. Aborting.
        if exist prop_boot_*.tmp del prop_boot_*.tmp
        pause
        exit /b
    )
    if exist prop_boot_*.tmp del prop_boot_*.tmp
    echo.
)

echo --- Re-signing vbmeta.img ---
echo [*] Verifying vbmeta key...
if "!PUBLIC_KEY!"=="2597c218aae470a130f61162feaae70afd97f011" (
    echo [+] Matched RSA4096 key.
    set "KEY_FILE=key\testkey_rsa4096.pem"
) else if "!PUBLIC_KEY!"=="cdbb77177f731920bbe0a0f94f84d9038ae0617d" (
    echo [+] Matched RSA2048 key.
    set "KEY_FILE=key\testkey_rsa2048.pem"
) else (
    echo [!] Public key '!PUBLIC_KEY!' did not match known keys. Aborting.
    pause
    exit /b
)

if not defined ALGORITHM (
    echo [!] Could not determine Algorithm from 'vbmeta.img'. Aborting.
    pause
    exit /b
)
echo.
echo [*] Re-signing 'vbmeta.img' using backup descriptors...
set "PADDING_SIZE=8192"

"%PYTHON%" "%PY_AVBTOOL%" make_vbmeta_image ^
	--output vbmeta.img ^
	--key "%~dp0!KEY_FILE!" ^
	--algorithm !ALGORITHM! ^
	--padding_size !PADDING_SIZE! ^
	--include_descriptors_from_image vbmeta.bak.img ^
	--include_descriptors_from_image vendor_boot_prc.img

if errorlevel 1 (
    echo [!] Failed to re-sign 'vbmeta.img'. Aborting.
    pause
    exit /b
)
echo.
echo --- Finalizing ---
echo [*] Renaming final images...
if exist vendor_boot_prc.img (
    ren vendor_boot_prc.img vendor_boot.img
)
if defined PROCESS_BOOT (
    ren boot.root.img boot.img
)
echo.
echo [*] Moving final images to 'output' folder...
if not exist output mkdir output
move vendor_boot.img output
move vbmeta.img output
if defined PROCESS_BOOT move boot.img output
echo.
echo [*] Moving backup files to 'backup' folder...
if not exist backup mkdir backup
move *.bak.img backup > nul
echo.
echo =============================================================
echo  SUCCESS!
echo  Final images have been saved to the 'output' folder.
echo =============================================================
echo.
pause