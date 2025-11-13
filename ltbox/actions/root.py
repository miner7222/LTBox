import os
import platform
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Optional, Dict

from ..constants import *
from .. import utils, device, downloader
from ..downloader import ensure_magiskboot
from .xml import _ensure_params_or_fail
from .system import detect_active_slot_robust
from .edl import _fh_loader_write_part
from ..patch.root import patch_boot_with_root_algo
from ..patch.avb import process_boot_image_avb
from ..i18n import get_string

def root_boot_only() -> None:
    print(get_string("act_clean_root_out").format(dir=OUTPUT_ROOT_DIR.name))
    if OUTPUT_ROOT_DIR.exists():
        shutil.rmtree(OUTPUT_ROOT_DIR)
    OUTPUT_ROOT_DIR.mkdir(exist_ok=True)
    print()
    
    utils.check_dependencies()
    magiskboot_exe = utils.get_platform_executable("magiskboot")
    ensure_magiskboot()

    if platform.system() != "Windows":
        os.chmod(magiskboot_exe, 0o755)

    print(get_string("act_wait_boot"))
    IMAGE_DIR.mkdir(exist_ok=True) 
    required_files = ["boot.img"]
    prompt = get_string("act_prompt_boot").format(name=IMAGE_DIR.name)
    utils.wait_for_files(IMAGE_DIR, required_files, prompt)
    
    boot_img_src = IMAGE_DIR / "boot.img"
    boot_img = BASE_DIR / "boot.img" 
    
    try:
        shutil.copy(boot_img_src, boot_img)
        print(get_string("act_copy_boot").format(name=boot_img_src.name))
    except (IOError, OSError) as e:
        print(get_string("act_err_copy_boot").format(name=boot_img_src.name, e=e), file=sys.stderr)
        sys.exit(1)

    if not boot_img.exists():
        print(get_string("act_err_boot_missing"))
        sys.exit(1)

    shutil.copy(boot_img, BASE_DIR / "boot.bak.img")
    print(get_string("act_backup_boot"))

    with utils.temporary_workspace(WORK_DIR):
        shutil.copy(boot_img, WORK_DIR / "boot.img")
        boot_img.unlink()
        
        patched_boot_path = patch_boot_with_root_algo(WORK_DIR, magiskboot_exe)

        if patched_boot_path and patched_boot_path.exists():
            print(get_string("act_finalize_root"))
            final_boot_img = OUTPUT_ROOT_DIR / "boot.img"
            
            process_boot_image_avb(patched_boot_path)

            print(get_string("act_move_root_final").format(dir=OUTPUT_ROOT_DIR.name))
            shutil.move(patched_boot_path, final_boot_img)

            print(get_string("act_move_root_backup").format(dir=BACKUP_DIR.name))
            BACKUP_DIR.mkdir(exist_ok=True)
            for bak_file in BASE_DIR.glob("boot.bak.img"):
                shutil.move(bak_file, BACKUP_DIR / bak_file.name)
            print()

            print("=" * 61)
            print(get_string("act_success"))
            print(get_string("act_root_saved").format(dir=OUTPUT_ROOT_DIR.name))
            print("=" * 61)
        else:
            print(get_string("act_err_root_fail"), file=sys.stderr)

def root_device(skip_adb=False) -> None:
    print(get_string("act_start_root"))
    
    if OUTPUT_ROOT_DIR.exists():
        shutil.rmtree(OUTPUT_ROOT_DIR)
    OUTPUT_ROOT_DIR.mkdir(exist_ok=True)
    BACKUP_BOOT_DIR.mkdir(exist_ok=True)

    utils.check_dependencies()
    
    magiskboot_exe = utils.get_platform_executable("magiskboot")
    ensure_magiskboot()

    dev = device.DeviceController(skip_adb=skip_adb)

    print(get_string("act_root_step1"))
    if not skip_adb:
        dev.wait_for_adb()

    active_slot = detect_active_slot_robust(dev, skip_adb)

    if active_slot:
        print(get_string("act_slot_confirmed").format(slot=active_slot))
        target_partition = f"boot{active_slot}"
    else:
        print(get_string("act_warn_root_slot"))
        target_partition = "boot"

    if not skip_adb:
        print(get_string("act_check_ksu"))
        downloader.download_ksu_apk(BASE_DIR)
        
        ksu_apks = list(BASE_DIR.glob("*spoofed*.apk"))
        if ksu_apks:
            apk_path = ksu_apks[0]
            print(get_string("act_install_ksu").format(name=apk_path.name))
            try:
                utils.run_command([str(ADB_EXE), "install", "-r", str(apk_path)])
                print(get_string("act_ksu_ok"))
            except Exception as e:
                print(get_string("act_err_ksu").format(e=e))
                print(get_string("act_root_anyway"))
        else:
            print(get_string("act_skip_ksu"))
    
    print(get_string("act_root_step2"))
    port = dev.setup_edl_connection()
    
    try:
        dev.load_firehose_programmer_with_stability(EDL_LOADER_FILE, port)
    except Exception as e:
        print(get_string("act_warn_prog_load").format(e=e))

    print(get_string("act_root_step3").format(part=target_partition))
    
    params = None
    final_boot_img = OUTPUT_ROOT_DIR / "boot.img"
    
    with utils.temporary_workspace(WORKING_BOOT_DIR):
        dumped_boot_img = WORKING_BOOT_DIR / "boot.img"
        backup_boot_img = BACKUP_BOOT_DIR / "boot.img"
        base_boot_bak = BASE_DIR / "boot.bak.img"

        try:
            params = _ensure_params_or_fail(target_partition)
            print(get_string("act_found_dump_info").format(xml=params['source_xml'], lun=params['lun'], start=params['start_sector']))
            dev.fh_loader_read_part(
                port=port,
                output_filename=str(dumped_boot_img),
                lun=params['lun'],
                start_sector=params['start_sector'],
                num_sectors=params['num_sectors']
            )
            print(get_string("act_read_boot_ok").format(part=target_partition, file=dumped_boot_img))
        except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
            print(get_string("act_err_dump").format(part=target_partition, e=e), file=sys.stderr)
            raise

        print(get_string("act_backup_boot_root").format(dir=backup_boot_img.parent.name))
        shutil.copy(dumped_boot_img, backup_boot_img)
        print(get_string("act_temp_backup_avb"))
        shutil.copy(dumped_boot_img, base_boot_bak)
        print(get_string("act_backups_done"))

        print(get_string("act_dump_reset"))
        dev.fh_loader_reset(port)
        
        print(get_string("act_root_step4"))
        patched_boot_path = patch_boot_with_root_algo(WORKING_BOOT_DIR, magiskboot_exe)

        if not (patched_boot_path and patched_boot_path.exists()):
            print(get_string("act_err_root_fail"), file=sys.stderr)
            base_boot_bak.unlink(missing_ok=True)
            sys.exit(1)

        print(get_string("act_root_step5"))
        try:
            process_boot_image_avb(patched_boot_path)
        except Exception as e:
            print(get_string("act_err_avb_footer").format(e=e), file=sys.stderr)
            base_boot_bak.unlink(missing_ok=True)
            raise

        shutil.move(patched_boot_path, final_boot_img)
        print(get_string("act_patched_boot_saved").format(dir=final_boot_img.parent.name))

        base_boot_bak.unlink(missing_ok=True)

    print(get_string("act_root_step6").format(part=target_partition))
    
    if not skip_adb:
        print(get_string("act_wait_sys_adb"))
        dev.wait_for_adb()
        print(get_string("act_reboot_edl_flash"))
        port = dev.setup_edl_connection()
    else:
        print(get_string("act_skip_adb_on"))
        print(get_string("act_manual_edl_now"))
        port = dev.wait_for_edl()

    try:
        dev.load_firehose_programmer_with_stability(EDL_LOADER_FILE, port)
    except Exception as e:
        print(get_string("act_warn_prog_load").format(e=e))

    if not params:
         params = _ensure_params_or_fail(target_partition)

    try:
        _fh_loader_write_part(
            port=port,
            image_path=final_boot_img,
            lun=params['lun'],
            start_sector=params['start_sector']
        )
        print(get_string("act_flash_boot_ok").format(part=target_partition))
        
        print(get_string("act_reset_sys"))
        dev.fh_loader_reset(port)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(get_string("act_err_edl_write").format(e=e), file=sys.stderr)
        raise

    print(get_string("act_root_finish"))

def unroot_device(skip_adb=False) -> None:
    print(get_string("act_start_unroot"))
    
    backup_boot_file = BACKUP_BOOT_DIR / "boot.img"
    BACKUP_BOOT_DIR.mkdir(exist_ok=True)
    
    print(get_string("act_unroot_step1"))
    if not list(IMAGE_DIR.glob("rawprogram*.xml")) and not list(IMAGE_DIR.glob("*.x")):
         print(get_string("act_err_no_xmls").format(dir=IMAGE_DIR.name))
         print(get_string("act_unroot_req_xmls"))
         prompt = get_string("act_prompt_image")
         utils.wait_for_directory(IMAGE_DIR, prompt)

    print(get_string("act_unroot_step2"))
    if not backup_boot_file.exists():
        prompt = get_string("act_prompt_backup_boot").format(dir=BACKUP_BOOT_DIR.name)
        utils.wait_for_files(BACKUP_BOOT_DIR, ["boot.img"], prompt)
    
    print(get_string("act_backup_boot_found"))

    dev = device.DeviceController(skip_adb=skip_adb)
    target_partition = "boot"

    print(get_string("act_unroot_step3"))
    if not skip_adb:
        dev.wait_for_adb()
    
    active_slot = detect_active_slot_robust(dev, skip_adb)
    
    if active_slot:
        print(get_string("act_slot_confirmed").format(slot=active_slot))
        target_partition = f"boot{active_slot}"
    else:
        print(get_string("act_warn_unroot_slot"))

    port = dev.setup_edl_connection()

    try:
        dev.load_firehose_programmer_with_stability(EDL_LOADER_FILE, port)
    except Exception as e:
        print(get_string("act_warn_prog_load").format(e=e))

    print(get_string("act_unroot_step4").format(part=target_partition))
    try:
        params = _ensure_params_or_fail(target_partition)
        print(get_string("act_found_dump_info").format(xml=params['source_xml'], lun=params['lun'], start=params['start_sector']))
        
        _fh_loader_write_part(
            port=port,
            image_path=backup_boot_file,
            lun=params['lun'],
            start_sector=params['start_sector']
        )
        print(get_string("act_flash_stock_boot_ok").format(part=target_partition))
        
        print(get_string("act_reset_sys"))
        dev.fh_loader_reset(port)
        
    except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
        print(get_string("act_err_edl_write").format(e=e), file=sys.stderr)
        raise

    print(get_string("act_unroot_finish"))