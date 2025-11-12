import subprocess
import sys
import shutil
from typing import Optional

from ltbox.constants import *
from ltbox import utils, device, actions

def patch_all(wipe: int = 0, skip_adb: bool = False) -> None:
    
    print("--- [STEP 1/9] Cleaning up previous output folders ---")
    output_folders_to_clean = [
        OUTPUT_DIR, 
        OUTPUT_ROOT_DIR, 
        OUTPUT_DP_DIR, 
        OUTPUT_ANTI_ROLLBACK_DIR,
        OUTPUT_XML_DIR
    ]
    
    for folder in output_folders_to_clean:
        if folder.exists():
            try:
                shutil.rmtree(folder)
                print(f"  > Removed: {folder.name}")
            except OSError as e:
                print(f"[!] Error removing {folder.name}: {e}", file=sys.stderr)

    if wipe == 1:
        print("\n--- [WIPE MODE] Starting Automated Install & Flash ROW Firmware Process ---")
    else:
        print("\n--- [NO WIPE MODE] Starting Automated Update & Flash ROW Firmware Process ---")
    
    print("\n" + "="*61)
    print("  STEP 2/9: Waiting for ADB/Fastboot Connection & Getting Device Info")
    print("="*61)
    
    dev = device.DeviceController(skip_adb=skip_adb)
    device_model: Optional[str] = None
    active_slot_suffix: Optional[str] = None

    if not skip_adb:
        dev.wait_for_adb()
        device_model = dev.get_device_model()
        if not device_model:
            raise SystemExit("Failed to get device model via ADB.")
        active_slot_suffix = dev.get_active_slot_suffix()
        
        print("\n--- [STEP 2/9] Device Info Check SUCCESS ---")
        print("[*] Device will boot back to system if needed. Waiting for ADB connection again...")
        dev.wait_for_adb()
        print("[+] ADB device connected.")

    else:
        print("[!] Skip ADB is ON. Switching to Fastboot for slot detection.")
        print("    Please manually boot your device into FASTBOOT mode.")
        dev.wait_for_fastboot()
        
        active_slot_suffix = dev.get_active_slot_suffix_from_fastboot()
        
        print("\n--- [STEP 2/9] Device Info Check (Fastboot) FINISHED ---")
        print("[*] Note: Device is currently in Fastboot mode.")
        print("[*] For the next steps, you will need to switch to EDL mode manually")
        print("    or rely on the script waiting for EDL connection.")

    print("\n--- [STEP 3/9] Waiting for RSA Firmware 'image' folder ---")
    prompt = (
        "Please copy the entire 'image' folder from your\n"
        "         unpacked Lenovo RSA firmware into the main directory.\n"
        r"         (Typical Location: C:\ProgramData\RSA\Download\RomFiles\...)"
    )
    utils.wait_for_directory(IMAGE_DIR, prompt)
    print("[+] 'image' folder found.")
    
    skip_dp_workflow = False
    
    try:
        print("\n" + "="*61)
        print("  STEP 4/9: Converting Firmware (PRC to ROW) & Validating Model")
        print("="*61)
        actions.convert_images(device_model=device_model, skip_adb=skip_adb)
        print("\n--- [STEP 4/9] Firmware Conversion & Validation SUCCESS ---")

        print("\n" + "="*61)
        print("  STEP 5/9: Modifying XML Files")
        print("="*61)
        actions.modify_xml(wipe=wipe)
        print("\n--- [STEP 5/9] XML Modification SUCCESS ---")
        
        print("\n" + "="*61)
        print("  STEP 6/9: Dumping devinfo/persist for patching (fh_loader)")
        print("="*61)

        suffix = active_slot_suffix if active_slot_suffix else ""
        boot_target = f"boot{suffix}"
        vbmeta_target = f"vbmeta_system{suffix}"
        
        extra_dumps = [boot_target, vbmeta_target]
        
        print(f"[*] Scheduled extra dumps for ARB check: {', '.join(extra_dumps)}")
        
        dump_status = actions.read_edl_fhloader(
            skip_adb=skip_adb, 
            skip_reset=False, 
            additional_targets=extra_dumps
        )

        if dump_status == "SKIP_DP":
            skip_dp_workflow = True
            print("[!] Skipping devinfo/persist patching and flashing steps.")
        print("\n--- [STEP 6/9] Dump SUCCESS ---")
        
        
        if not skip_dp_workflow:
            print("\n" + "="*61)
            print("  STEP 7/9: Patching devinfo/persist")
            print("="*61)
            actions.edit_devinfo_persist()
            print("\n--- [STEP 7/9] Patching SUCCESS ---")
        else:
            print("\n" + "="*61)
            print("  STEP 7/9: Patching devinfo/persist (SKIPPED)")
            print("="*61)

        
        print("\n" + "="*61)
        print("  STEP 8/9: Checking and Patching Anti-Rollback")
        print("="*61)
        
        print("[*] Using Dumped Images for ARB Check...")
        dumped_boot = BACKUP_DIR / f"{boot_target}.img"
        dumped_vbmeta = BACKUP_DIR / f"{vbmeta_target}.img"
        
        arb_status_result = actions.read_anti_rollback(
            dumped_boot_path=dumped_boot,
            dumped_vbmeta_path=dumped_vbmeta
        )
        
        if arb_status_result[0] == 'ERROR':
            print("\n" + "!"*61)
            print("  CRITICAL ERROR: Anti-Rollback check failed!")
            print("  Could not determine device security version.")
            print("  Aborting process to prevent bricking.")
            print("!"*61)
            sys.exit(1)

        actions.patch_anti_rollback(comparison_result=arb_status_result)
        print("\n--- [STEP 8/9] Anti-Rollback Check/Patch SUCCESS ---")
        
        print("\n" + "="*61)
        print("  [FINAL STEP 9/9] Flashing All Images via EDL")
        print("="*61)
        print("The device will now be flashed with all modified images.")
        actions.flash_edl(skip_reset_edl=True, skip_dp=skip_dp_workflow) 
        
        print("\n" + "=" * 61)
        print("  FULL PROCESS COMPLETE!")
        print("  Your device should now reboot with a patched ROW firmware.")
        print("=" * 61)

    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
        print("\n" + "!" * 61)
        print("  AN ERROR OCCURRED: Process Halted.")
        print(f"  Error details: {e}")
        print("!" * 61)
        sys.exit(1)
    except SystemExit as e:
        print("\n" + "!" * 61)
        print(f"  PROCESS HALTED BY SCRIPT: {e}")
        print("!" * 61)
    except KeyboardInterrupt:
        print("\n" + "!" * 61)
        print("  PROCESS CANCELLED BY USER.")
        print("!" * 61)