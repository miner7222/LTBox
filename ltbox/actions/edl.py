import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Optional, List, Dict

from ..constants import *
from .. import utils, device
from .xml import _ensure_params_or_fail
from ..i18n import get_string

def _fh_loader_write_part(port, image_path, lun, start_sector):
    if not FH_LOADER_EXE.exists():
        raise FileNotFoundError(get_string("act_err_fh_exe_missing").format(path=FH_LOADER_EXE))
        
    port_str = f"\\\\.\\{port}"
    cmd = [
        str(FH_LOADER_EXE),
        f"--port={port_str}",
        f"--sendimage={image_path}",
        f"--lun={lun}",
        f"--start_sector={start_sector}",
        "--zlpawarehost=1",
        "--noprompt",
        "--memoryname=UFS"
    ]
    print(get_string("act_flash_part").format(name=image_path.name, lun=lun, start=start_sector))
    utils.run_command(cmd)

def read_edl(skip_adb: bool = False, skip_reset: bool = False, additional_targets: Optional[List[str]] = None) -> None:
    print(get_string("act_start_dump"))
    
    dev = device.DeviceController(skip_adb=skip_adb)
    port = dev.setup_edl_connection()
    
    try:
        dev.load_firehose_programmer_with_stability(EDL_LOADER_FILE, port)
    except Exception as e:
        print(get_string("act_warn_prog_load").format(e=e))

    BACKUP_DIR.mkdir(exist_ok=True)
    
    targets = ["devinfo", "persist"]

    if additional_targets:
        targets.extend(additional_targets)
        print(get_string("act_ext_dump_targets").format(targets=', '.join(targets)))
    
    for target in targets:
        out_file = BACKUP_DIR / f"{target}.img"
        print(get_string("act_prep_dump").format(target=target))
        
        try:
            params = _ensure_params_or_fail(target)
            print(get_string("act_found_dump_info").format(xml=params['source_xml'], lun=params['lun'], start=params['start_sector']))
            
            dev.fh_loader_read_part(
                port=port,
                output_filename=str(out_file),
                lun=params['lun'],
                start_sector=params['start_sector'],
                num_sectors=params['num_sectors']
            )
            print(get_string("act_dump_success").format(target=target, file=out_file.name))
            
        except (ValueError, FileNotFoundError) as e:
            print(get_string("act_skip_dump").format(target=target, e=e))
        except Exception as e:
            print(get_string("act_err_dump").format(target=target, e=e), file=sys.stderr)

        print(get_string("act_wait_stability"))
        time.sleep(5)

    if not skip_reset:
        print(get_string("act_reset_sys"))
        dev.fh_loader_reset(port)
        print(get_string("act_reset_sent"))
        print(get_string("act_wait_stability_long"))
        time.sleep(10)
    else:
        print(get_string("act_skip_reset"))

    print(get_string("act_dump_finish"))
    print(get_string("act_dump_saved").format(dir=BACKUP_DIR.name))

def read_edl_fhloader(skip_adb: bool = False, skip_reset: bool = False, additional_targets: Optional[List[str]] = None) -> None:
    return read_edl(skip_adb, skip_reset=skip_reset, additional_targets=additional_targets)

def write_edl(skip_reset: bool = False, skip_reset_edl: bool = False) -> None:
    print(get_string("act_start_write"))

    skip_adb_val = os.environ.get('SKIP_ADB') == '1'
    dev = device.DeviceController(skip_adb=skip_adb_val)

    if not OUTPUT_DP_DIR.exists():
        print(get_string("act_err_dp_folder").format(dir=OUTPUT_DP_DIR.name), file=sys.stderr)
        print(get_string("act_err_run_patch_first"), file=sys.stderr)
        raise FileNotFoundError(get_string("act_err_dp_folder_nf").format(dir=OUTPUT_DP_DIR.name))
    print(get_string("act_found_dp_folder").format(dir=OUTPUT_DP_DIR.name))

    port = dev.setup_edl_connection()

    try:
        dev.load_firehose_programmer_with_stability(EDL_LOADER_FILE, port)
    except Exception as e:
        print(get_string("act_warn_prog_load").format(e=e))

    targets = ["devinfo", "persist"]

    for target in targets:
        image_path = OUTPUT_DP_DIR / f"{target}.img"

        if not image_path.exists():
            print(get_string(f"act_skip_{target}"))
            continue

        print(f"[*] Flashing '{target}' via EDL...")

        try:
            params = _ensure_params_or_fail(target)
            print(get_string("act_found_boot_info").format(lun=params['lun'], start=params['start_sector']))
            
            dev.fh_loader_write_part(
                port=port,
                image_path=image_path,
                lun=params['lun'],
                start_sector=params['start_sector']
            )
            print(get_string(f"act_flash_{target}_ok"))

        except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
            print(get_string("act_err_edl_write").format(e=e), file=sys.stderr)
            raise

    if not skip_reset:
        print(get_string("act_reboot_device"))
        try:
            dev.fh_loader_reset(port)
        except Exception as e:
            print(get_string("act_warn_reboot").format(e=e))
    else:
        print(get_string("act_skip_reboot"))

    print(get_string("act_write_finish"))

def write_anti_rollback(skip_reset: bool = False) -> None:
    print(get_string("act_start_arb_write"))

    boot_img = OUTPUT_ANTI_ROLLBACK_DIR / "boot.img"
    vbmeta_img = OUTPUT_ANTI_ROLLBACK_DIR / "vbmeta_system.img"

    if not boot_img.exists() or not vbmeta_img.exists():
        print(get_string("act_err_patched_missing").format(dir=OUTPUT_ANTI_ROLLBACK_DIR.name), file=sys.stderr)
        print(get_string("act_err_run_patch_arb"), file=sys.stderr)
        raise FileNotFoundError(get_string("act_err_patched_missing_exc").format(dir=OUTPUT_ANTI_ROLLBACK_DIR.name))
    print(get_string("act_found_arb_folder").format(dir=OUTPUT_ANTI_ROLLBACK_DIR.name))

    dev = device.DeviceController(skip_adb=True)
    
    print(get_string("act_arb_write_step1"))
    print(get_string("act_boot_fastboot"))
    dev.wait_for_fastboot()

    active_slot = dev.get_active_slot_suffix_from_fastboot()
    if active_slot:
        print(get_string("act_slot_confirmed").format(slot=active_slot))
    else:
        print(get_string("act_warn_slot_fail"))
        active_slot = ""

    target_boot = f"boot{active_slot}"
    target_vbmeta = f"vbmeta_system{active_slot}"

    print(get_string("act_arb_write_step2"))
    print(get_string("act_manual_edl_now"))
    print(get_string("act_manual_edl_hint"))
    port = dev.wait_for_edl()
    
    try:
        dev.load_firehose_programmer_with_stability(EDL_LOADER_FILE, port)

        print(get_string("act_arb_write_step3").format(slot=active_slot))

        print(get_string("act_write_boot").format(target=target_boot))
        params_boot = _ensure_params_or_fail(target_boot)
        print(get_string("act_found_boot_info").format(lun=params_boot['lun'], start=params_boot['start_sector']))
        dev.fh_loader_write_part(
            port=port,
            image_path=boot_img,
            lun=params_boot['lun'],
            start_sector=params_boot['start_sector']
        )
        print(get_string("act_write_boot_ok").format(target=target_boot))

        print(get_string("act_write_vbmeta").format(target=target_vbmeta))
        params_vbmeta = _ensure_params_or_fail(target_vbmeta)
        print(get_string("act_found_vbmeta_info").format(lun=params_vbmeta['lun'], start=params_vbmeta['start_sector']))
        dev.fh_loader_write_part(
            port=port,
            image_path=vbmeta_img,
            lun=params_vbmeta['lun'],
            start_sector=params_vbmeta['start_sector']
        )
        print(get_string("act_write_vbmeta_ok").format(target=target_vbmeta))

        if not skip_reset:
            print(get_string("act_arb_reset"))
            dev.fh_loader_reset(port)
            print(get_string("act_reset_sent"))
        else:
            print(get_string("act_arb_skip_reset"))

    except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
        print(get_string("act_err_edl_write").format(e=e), file=sys.stderr)
        raise
    
    print(get_string("act_arb_write_finish"))

def flash_edl(skip_reset: bool = False, skip_reset_edl: bool = False, skip_dp: bool = False) -> None:
    print(get_string("act_start_flash"))

    skip_adb_val = os.environ.get('SKIP_ADB') == '1'
    dev = device.DeviceController(skip_adb=skip_adb_val)
    
    if not IMAGE_DIR.is_dir() or not any(IMAGE_DIR.iterdir()):
        print(get_string("act_err_image_empty").format(dir=IMAGE_DIR.name))
        print(get_string("act_err_run_xml_mod"))
        raise FileNotFoundError(get_string("act_err_image_empty_exc").format(dir=IMAGE_DIR.name))
        
    loader_path = EDL_LOADER_FILE
    if not loader_path.exists():
        print(get_string("act_err_loader_missing").format(name=loader_path.name, dir=IMAGE_DIR.name))
        print(get_string("act_err_copy_loader"))
        raise FileNotFoundError(get_string("act_err_loader_missing_exc").format(name=loader_path.name, dir=IMAGE_DIR.name))

    if not skip_reset_edl:
        print("\n" + "="*61)
        print(get_string("act_warn_overwrite_1"))
        print(get_string("act_warn_overwrite_2"))
        print(get_string("act_warn_overwrite_3"))
        print("="*61 + "\n")
        
        choice = ""
        while choice not in ['y', 'n']:
            choice = input(get_string("act_ask_continue")).lower().strip()

        if choice == 'n':
            print(get_string("act_op_cancel"))
            return

    print(get_string("act_copy_patched"))
    output_folders_to_copy = [
        OUTPUT_DIR, 
        OUTPUT_ANTI_ROLLBACK_DIR,
        OUTPUT_XML_DIR
    ]
    
    copied_count = 0
    for folder in output_folders_to_copy:
        if folder.exists():
            try:
                shutil.copytree(folder, IMAGE_DIR, dirs_exist_ok=True)
                print(get_string("act_copied_content").format(src=folder.name, dst=IMAGE_DIR.name))
                copied_count += 1
            except Exception as e:
                print(get_string("act_err_copy").format(name=folder.name, e=e), file=sys.stderr)
    
    if not skip_dp:
        if OUTPUT_DP_DIR.exists():
            try:
                shutil.copytree(OUTPUT_DP_DIR, IMAGE_DIR, dirs_exist_ok=True)
                print(get_string("act_copied_dp").format(src=OUTPUT_DP_DIR.name, dst=IMAGE_DIR.name))
                copied_count += 1
            except Exception as e:
                print(get_string("act_err_copy_dp").format(name=OUTPUT_DP_DIR.name, e=e), file=sys.stderr)
        else:
            print(get_string("act_skip_dp_copy").format(dir=OUTPUT_DP_DIR.name))
    else:
        print(get_string("act_req_skip_dp"))

    if copied_count == 0:
        print(get_string("act_no_output_folders"))

    port = dev.setup_edl_connection()

    raw_xmls = [f for f in IMAGE_DIR.glob("rawprogram*.xml") if f.name != "rawprogram0.xml"]
    patch_xmls = list(IMAGE_DIR.glob("patch*.xml"))
    
    persist_write_xml = IMAGE_DIR / "rawprogram_write_persist_unsparse0.xml"
    persist_save_xml = IMAGE_DIR / "rawprogram_save_persist_unsparse0.xml"
    devinfo_write_xml = IMAGE_DIR / "rawprogram4_write_devinfo.xml"
    devinfo_original_xml = IMAGE_DIR / "rawprogram4.xml"

    has_patched_persist = (OUTPUT_DP_DIR / "persist.img").exists()
    has_patched_devinfo = (OUTPUT_DP_DIR / "devinfo.img").exists()

    if persist_write_xml.exists() and has_patched_persist and not skip_dp:
        print(get_string("act_use_patched_persist"))
        raw_xmls = [xml for xml in raw_xmls if xml.name != persist_save_xml.name]
    else:
        if persist_write_xml.exists() and any(xml.name == persist_write_xml.name for xml in raw_xmls):
             print(get_string("act_skip_persist_flash"))
             raw_xmls = [xml for xml in raw_xmls if xml.name != persist_write_xml.name]

    if devinfo_write_xml.exists() and has_patched_devinfo and not skip_dp:
        print(get_string("act_use_patched_devinfo"))
        raw_xmls = [xml for xml in raw_xmls if xml.name != devinfo_original_xml.name]
    else:
        if devinfo_write_xml.exists() and any(xml.name == devinfo_write_xml.name for xml in raw_xmls):
             print(get_string("act_skip_devinfo_flash"))
             raw_xmls = [xml for xml in raw_xmls if xml.name != devinfo_write_xml.name]

    if not raw_xmls or not patch_xmls:
        print(get_string("act_err_xml_missing").format(dir=IMAGE_DIR.name))
        print(get_string("act_err_flash_aborted"))
        raise FileNotFoundError(get_string("act_err_xml_missing_exc").format(dir=IMAGE_DIR.name))
        
    print(get_string("act_flash_step1"))
    
    try:
        dev.edl_rawprogram(loader_path, "UFS", raw_xmls, patch_xmls, port)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(get_string("act_err_main_flash").format(e=e), file=sys.stderr)
        print(get_string("act_warn_unstable"))
        raise
        
    print(get_string("act_flash_step2"))
    if not skip_dp:
        try:
            (IMAGE_DIR / "devinfo.img").unlink(missing_ok=True)
            (IMAGE_DIR / "persist.img").unlink(missing_ok=True)
            print(get_string("act_removed_temp_imgs"))
        except OSError as e:
            print(get_string("act_err_clean_imgs").format(e=e), file=sys.stderr)

    if not skip_reset:
        print(get_string("act_flash_step3"))
        try:
            print(get_string("act_wait_stability"))
            time.sleep(5)
            
            print(get_string("act_reset_sys"))
            dev.fh_loader_reset(port)
            print(get_string("act_reset_sent"))
        except (subprocess.CalledProcessError, FileNotFoundError, Exception) as e:
             print(get_string("act_err_reset").format(e=e), file=sys.stderr)
    else:
        print(get_string("act_skip_final_reset"))

    if not skip_reset:
        print(get_string("act_flash_finish"))