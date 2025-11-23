import os
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Optional, List, Tuple

from .. import constants as const
from .. import utils, device
from ..partition import ensure_params_or_fail
from ..i18n import get_string

def _prepare_edl_session(dev: device.DeviceController) -> str:
    if not const.EDL_LOADER_FILE.exists():
        utils.ui.echo(get_string("act_err_loader_missing").format(name=const.EDL_LOADER_FILE.name, dir=const.IMAGE_DIR.name))
        prompt = get_string("device_loader_prompt").format(loader=const.EDL_LOADER_FILENAME, folder=const.IMAGE_DIR.name)
        utils.wait_for_files(const.IMAGE_DIR, [const.EDL_LOADER_FILENAME], prompt)

    if not list(const.OUTPUT_XML_DIR.glob("rawprogram*.xml")) and not list(const.IMAGE_DIR.glob("rawprogram*.xml")) and not list(const.IMAGE_DIR.glob("*.x")):
         utils.ui.echo(get_string("act_err_no_xmls").format(dir=const.IMAGE_DIR.name))
         prompt = get_string("act_prompt_image")
         utils.wait_for_directory(const.IMAGE_DIR, prompt)

    port = dev.setup_edl_connection()
    
    try:
        dev.load_firehose_programmer_with_stability(const.EDL_LOADER_FILE, port)
    except Exception as e:
        utils.ui.echo(get_string("act_warn_prog_load").format(e=e))
        
    return port

def dump_partitions(dev: device.DeviceController, skip_reset: bool = False, additional_targets: Optional[List[str]] = None, default_targets: bool = True) -> None:
    utils.ui.echo(get_string("act_start_dump"))
    
    port = _prepare_edl_session(dev)

    const.BACKUP_DIR.mkdir(exist_ok=True)
    
    targets = []
    if default_targets:
        targets.extend(["devinfo", "persist"])

    if additional_targets:
        targets.extend(additional_targets)
        utils.ui.echo(get_string("act_ext_dump_targets").format(targets=', '.join(targets)))
    
    for target in targets:
        out_file = const.BACKUP_DIR / f"{target}.img"
        utils.ui.echo(get_string("act_prep_dump").format(target=target))
        
        try:
            params = ensure_params_or_fail(target)
            utils.ui.echo(get_string("act_found_dump_info").format(xml=params['source_xml'], lun=params['lun'], start=params['start_sector']))
            
            utils.ui.echo(get_string("device_dumping_part").format(lun=params['lun'], start=params['start_sector'], num=params['num_sectors']))

            dev.edl_read_partition(
                port=port,
                output_filename=str(out_file),
                lun=params['lun'],
                start_sector=params['start_sector'],
                num_sectors=params['num_sectors']
            )
            
            if params.get('size_in_kb'):
                try:
                    expected_size_bytes = int(float(params['size_in_kb']) * 1024)
                    actual_size_bytes = out_file.stat().st_size
                    
                    if expected_size_bytes != actual_size_bytes:
                        raise RuntimeError(
                            get_string("act_err_dump_size_mismatch").format(
                                target=target,
                                expected=expected_size_bytes,
                                actual=actual_size_bytes
                            )
                        )
                except (ValueError, OSError) as e:
                    utils.ui.echo(get_string("act_skip_dump").format(target=target, e=f"Size validation error: {e}"))

            utils.ui.echo(get_string("act_dump_success").format(target=target, file=out_file.name))
            
        except (ValueError, FileNotFoundError) as e:
            utils.ui.echo(get_string("act_skip_dump").format(target=target, e=e))
        except Exception as e:
            utils.ui.error(get_string("act_err_dump").format(target=target, e=e))

        utils.ui.echo(get_string("act_wait_stability"))
        time.sleep(5)

    utils.ui.echo(get_string("act_dump_ignore_warn"))
    utils.ui.echo(get_string("act_dump_finish"))
    utils.ui.echo(get_string("act_dump_saved").format(dir=const.BACKUP_DIR.name))

    if not skip_reset:
        utils.ui.echo(get_string("act_reset_sys"))
        utils.ui.echo(get_string("device_resetting"))
        dev.edl_reset(port)
        utils.ui.echo(get_string("act_reset_sent"))
        utils.ui.echo(get_string("act_wait_stability_long"))
        time.sleep(10)
    else:
        utils.ui.echo(get_string("act_skip_reset"))

def flash_partitions(dev: device.DeviceController, skip_reset: bool = False, skip_reset_edl: bool = False) -> None:
    utils.ui.echo(get_string("act_start_write"))

    if not const.OUTPUT_DP_DIR.exists():
        utils.ui.error(get_string("act_err_dp_folder").format(dir=const.OUTPUT_DP_DIR.name))
        utils.ui.error(get_string("act_err_run_patch_first"))
        raise FileNotFoundError(get_string("act_err_dp_folder_nf").format(dir=const.OUTPUT_DP_DIR.name))
    utils.ui.echo(get_string("act_found_dp_folder").format(dir=const.OUTPUT_DP_DIR.name))

    port = _prepare_edl_session(dev)

    targets = ["devinfo", "persist"]

    for target in targets:
        image_path = const.OUTPUT_DP_DIR / f"{target}.img"

        if not image_path.exists():
            utils.ui.echo(get_string(f"act_skip_{target}"))
            continue

        utils.ui.echo(get_string("act_flashing_target").format(target=target))

        try:
            params = ensure_params_or_fail(target)
            utils.ui.echo(get_string("act_found_boot_info").format(lun=params['lun'], start=params['start_sector']))
            
            utils.ui.echo(get_string("device_flashing_part").format(filename=image_path.name, lun=params['lun'], start=params['start_sector']))
            dev.edl_write_partition(
                port=port,
                image_path=image_path,
                lun=params['lun'],
                start_sector=params['start_sector']
            )
            utils.ui.echo(get_string(f"act_flash_{target}_ok"))

        except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
            utils.ui.error(get_string("act_err_edl_write").format(e=e))
            raise

    if not skip_reset:
        utils.ui.echo(get_string("act_reboot_device"))
        try:
            utils.ui.echo(get_string("device_resetting"))
            dev.edl_reset(port)
        except Exception as e:
            utils.ui.echo(get_string("act_warn_reboot").format(e=e))
    else:
        utils.ui.echo(get_string("act_skip_reboot"))

    utils.ui.echo(get_string("act_write_finish"))

def write_anti_rollback(dev: device.DeviceController, skip_reset: bool = False) -> None:
    utils.ui.echo(get_string("act_start_arb_write"))

    boot_img = const.OUTPUT_ANTI_ROLLBACK_DIR / "boot.img"
    vbmeta_img = const.OUTPUT_ANTI_ROLLBACK_DIR / "vbmeta_system.img"

    if not boot_img.exists() or not vbmeta_img.exists():
        utils.ui.error(get_string("act_err_patched_missing").format(dir=const.OUTPUT_ANTI_ROLLBACK_DIR.name))
        utils.ui.error(get_string("act_err_run_patch_arb"))
        raise FileNotFoundError(get_string("act_err_patched_missing_exc").format(dir=const.OUTPUT_ANTI_ROLLBACK_DIR.name))
    utils.ui.echo(get_string("act_found_arb_folder").format(dir=const.OUTPUT_ANTI_ROLLBACK_DIR.name))
    
    if not const.EDL_LOADER_FILE.exists():
        utils.ui.echo(get_string("act_err_loader_missing").format(name=const.EDL_LOADER_FILE.name, dir=const.IMAGE_DIR.name))
        prompt = get_string("device_loader_prompt").format(loader=const.EDL_LOADER_FILENAME, folder=const.IMAGE_DIR.name)
        utils.wait_for_files(const.IMAGE_DIR, [const.EDL_LOADER_FILENAME], prompt)

    if not list(const.OUTPUT_XML_DIR.glob("rawprogram*.xml")) and not list(const.IMAGE_DIR.glob("rawprogram*.xml")) and not list(const.IMAGE_DIR.glob("*.x")):
         utils.ui.echo(get_string("act_err_no_xmls").format(dir=const.IMAGE_DIR.name))
         prompt = get_string("act_prompt_image")
         utils.wait_for_directory(const.IMAGE_DIR, prompt)
    
    utils.ui.echo(get_string("act_arb_write_step1"))
    utils.ui.echo(get_string("act_boot_fastboot"))
    dev.wait_for_fastboot()

    utils.ui.echo(get_string("device_get_slot_fastboot"))
    active_slot = dev.get_active_slot_suffix_from_fastboot()
    if active_slot:
        utils.ui.echo(get_string("act_slot_confirmed").format(slot=active_slot))
    else:
        utils.ui.echo(get_string("act_warn_slot_fail"))
        active_slot = ""

    target_boot = f"boot{active_slot}"
    target_vbmeta = f"vbmeta_system{active_slot}"

    utils.ui.echo(get_string("act_arb_write_step2"))
    utils.ui.echo(get_string("act_manual_edl_now"))
    utils.ui.echo(get_string("act_manual_edl_hint"))
    port = dev.wait_for_edl()
    
    try:
        dev.load_firehose_programmer_with_stability(const.EDL_LOADER_FILE, port)

        utils.ui.echo(get_string("act_arb_write_step3").format(slot=active_slot))

        utils.ui.echo(get_string("act_write_boot").format(target=target_boot))
        params_boot = ensure_params_or_fail(target_boot)
        utils.ui.echo(get_string("act_found_boot_info").format(lun=params_boot['lun'], start=params_boot['start_sector']))
        
        utils.ui.echo(get_string("device_flashing_part").format(filename=boot_img.name, lun=params_boot['lun'], start=params_boot['start_sector']))
        dev.edl_write_partition(
            port=port,
            image_path=boot_img,
            lun=params_boot['lun'],
            start_sector=params_boot['start_sector']
        )
        utils.ui.echo(get_string("act_write_boot_ok").format(target=target_boot))

        utils.ui.echo(get_string("act_write_vbmeta").format(target=target_vbmeta))
        params_vbmeta = ensure_params_or_fail(target_vbmeta)
        utils.ui.echo(get_string("act_found_vbmeta_info").format(lun=params_vbmeta['lun'], start=params_vbmeta['start_sector']))
        
        utils.ui.echo(get_string("device_flashing_part").format(filename=vbmeta_img.name, lun=params_vbmeta['lun'], start=params_vbmeta['start_sector']))
        dev.edl_write_partition(
            port=port,
            image_path=vbmeta_img,
            lun=params_vbmeta['lun'],
            start_sector=params_vbmeta['start_sector']
        )
        utils.ui.echo(get_string("act_write_vbmeta_ok").format(target=target_vbmeta))

        if not skip_reset:
            utils.ui.echo(get_string("act_arb_reset"))
            utils.ui.echo(get_string("device_resetting"))
            dev.edl_reset(port)
            utils.ui.echo(get_string("act_reset_sent"))
        else:
            utils.ui.echo(get_string("act_arb_skip_reset"))

    except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
        utils.ui.error(get_string("act_err_edl_write").format(e=e))
        raise
    
    utils.ui.echo(get_string("act_arb_write_finish"))

def _prepare_flash_files(skip_dp: bool = False) -> None:
    utils.ui.echo(get_string("act_copy_patched"))
    output_folders_to_copy = [
        const.OUTPUT_DIR, 
        const.OUTPUT_ANTI_ROLLBACK_DIR,
        const.OUTPUT_XML_DIR
    ]
    
    copied_count = 0
    for folder in output_folders_to_copy:
        if folder.exists():
            try:
                shutil.copytree(folder, const.IMAGE_DIR, dirs_exist_ok=True)
                utils.ui.echo(get_string("act_copied_content").format(src=folder.name, dst=const.IMAGE_DIR.name))
                copied_count += 1
            except Exception as e:
                utils.ui.error(get_string("act_err_copy").format(name=folder.name, e=e))
    
    if not skip_dp:
        if const.OUTPUT_DP_DIR.exists():
            try:
                shutil.copytree(const.OUTPUT_DP_DIR, const.IMAGE_DIR, dirs_exist_ok=True)
                utils.ui.echo(get_string("act_copied_dp").format(src=const.OUTPUT_DP_DIR.name, dst=const.IMAGE_DIR.name))
                copied_count += 1
            except Exception as e:
                utils.ui.error(get_string("act_err_copy_dp").format(name=const.OUTPUT_DP_DIR.name, e=e))
        else:
            utils.ui.echo(get_string("act_skip_dp_copy").format(dir=const.OUTPUT_DP_DIR.name))
    else:
        utils.ui.echo(get_string("act_req_skip_dp"))

    if copied_count == 0:
        utils.ui.echo(get_string("act_no_output_folders"))

def _select_flash_xmls(skip_dp: bool = False) -> Tuple[List[Path], List[Path]]:
    raw_xmls = [f for f in const.IMAGE_DIR.glob("rawprogram*.xml") if f.name != "rawprogram0.xml"]
    patch_xmls = list(const.IMAGE_DIR.glob("patch*.xml"))
    
    persist_write_xml = const.IMAGE_DIR / "rawprogram_write_persist_unsparse0.xml"
    persist_save_xml = const.IMAGE_DIR / "rawprogram_save_persist_unsparse0.xml"
    devinfo_write_xml = const.IMAGE_DIR / "rawprogram4_write_devinfo.xml"
    devinfo_original_xml = const.IMAGE_DIR / "rawprogram4.xml"

    has_patched_persist = (const.OUTPUT_DP_DIR / "persist.img").exists()
    has_patched_devinfo = (const.OUTPUT_DP_DIR / "devinfo.img").exists()

    if persist_write_xml.exists() and has_patched_persist and not skip_dp:
        utils.ui.echo(get_string("act_use_patched_persist"))
        raw_xmls = [xml for xml in raw_xmls if xml.name != persist_save_xml.name]
    else:
        if persist_write_xml.exists() and any(xml.name == persist_write_xml.name for xml in raw_xmls):
             utils.ui.echo(get_string("act_skip_persist_flash"))
             raw_xmls = [xml for xml in raw_xmls if xml.name != persist_write_xml.name]

    if devinfo_write_xml.exists() and has_patched_devinfo and not skip_dp:
        utils.ui.echo(get_string("act_use_patched_devinfo"))
        raw_xmls = [xml for xml in raw_xmls if xml.name != devinfo_original_xml.name]
    else:
        if devinfo_write_xml.exists() and any(xml.name == devinfo_write_xml.name for xml in raw_xmls):
             utils.ui.echo(get_string("act_skip_devinfo_flash"))
             raw_xmls = [xml for xml in raw_xmls if xml.name != devinfo_write_xml.name]

    if not raw_xmls or not patch_xmls:
        utils.ui.echo(get_string("act_err_xml_missing").format(dir=const.IMAGE_DIR.name))
        utils.ui.echo(get_string("act_err_flash_aborted"))
        raise FileNotFoundError(get_string("act_err_xml_missing_exc").format(dir=const.IMAGE_DIR.name))
    
    return raw_xmls, patch_xmls

def flash_full_firmware(dev: device.DeviceController, skip_reset: bool = False, skip_reset_edl: bool = False, skip_dp: bool = False) -> None:
    utils.ui.echo(get_string("act_start_flash"))
    
    if not const.IMAGE_DIR.is_dir() or not any(const.IMAGE_DIR.iterdir()):
        utils.ui.echo(get_string("act_err_image_empty").format(dir=const.IMAGE_DIR.name))
        utils.ui.echo(get_string("act_err_run_xml_mod"))
        raise FileNotFoundError(get_string("act_err_image_empty_exc").format(dir=const.IMAGE_DIR.name))
        
    loader_path = const.EDL_LOADER_FILE
    if not loader_path.exists():
        utils.ui.echo(get_string("act_err_loader_missing").format(name=loader_path.name, dir=const.IMAGE_DIR.name))
        utils.ui.echo(get_string("act_err_copy_loader"))
        raise FileNotFoundError(get_string("device_err_fh_missing").format(path=loader_path.name, dir=const.IMAGE_DIR.name))

    if not skip_reset_edl:
        utils.ui.echo("\n" + "="*61)
        utils.ui.echo(get_string("act_warn_overwrite_1"))
        utils.ui.echo(get_string("act_warn_overwrite_2"))
        utils.ui.echo(get_string("act_warn_overwrite_3"))
        utils.ui.echo("="*61 + "\n")
        
        choice = ""
        while choice not in ['y', 'n']:
            choice = utils.ui.prompt(get_string("act_ask_continue")).lower().strip()

        if choice == 'n':
            utils.ui.echo(get_string("act_op_cancel"))
            return

    _prepare_flash_files(skip_dp)

    port = dev.setup_edl_connection()

    raw_xmls, patch_xmls = _select_flash_xmls(skip_dp)
        
    utils.ui.echo(get_string("act_flash_step1"))
    
    try:
        dev.edl_rawprogram(loader_path, "UFS", raw_xmls, patch_xmls, port)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        utils.ui.error(get_string("act_err_main_flash").format(e=e))
        utils.ui.echo(get_string("act_warn_unstable"))
        raise
        
    utils.ui.echo(get_string("act_flash_step2"))
    if not skip_dp:
        try:
            (const.IMAGE_DIR / "devinfo.img").unlink(missing_ok=True)
            (const.IMAGE_DIR / "persist.img").unlink(missing_ok=True)
            utils.ui.echo(get_string("act_removed_temp_imgs"))
        except OSError as e:
            utils.ui.error(get_string("act_err_clean_imgs").format(e=e))

    if not skip_reset:
        utils.ui.echo(get_string("act_flash_step3"))
        try:
            utils.ui.echo(get_string("act_wait_stability"))
            time.sleep(5)
            
            utils.ui.echo(get_string("act_reset_sys"))
            utils.ui.echo(get_string("device_resetting"))
            dev.edl_reset(port)
            utils.ui.echo(get_string("act_reset_sent"))
        except (subprocess.CalledProcessError, FileNotFoundError, Exception) as e:
             utils.ui.error(get_string("act_err_reset").format(e=e))
    else:
        utils.ui.echo(get_string("act_skip_final_reset"))

    if not skip_reset:
        utils.ui.echo(get_string("act_flash_finish"))