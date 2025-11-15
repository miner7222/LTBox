import subprocess
import sys
import shutil
from typing import Optional, Tuple

from . import constants as const
from . import utils, device, actions
from .i18n import get_string

def _cleanup_previous_outputs(wipe: int) -> None:
    utils.ui.echo(get_string('wf_step1_clean'))
    output_folders_to_clean = [
        const.OUTPUT_DIR, 
        const.OUTPUT_ROOT_DIR, 
        const.OUTPUT_DP_DIR, 
        const.OUTPUT_ANTI_ROLLBACK_DIR,
        const.OUTPUT_XML_DIR
    ]
    
    for folder in output_folders_to_clean:
        if folder.exists():
            try:
                shutil.rmtree(folder)
                utils.ui.echo(get_string('wf_removed').format(name=folder.name))
            except OSError as e:
                utils.ui.echo(get_string('wf_remove_error').format(name=folder.name, e=e), err=True)

    if wipe == 1:
        utils.ui.echo(get_string('wf_wipe_mode_start'))
    else:
        utils.ui.echo(get_string('wf_nowipe_mode_start'))

def _get_device_info(dev: device.DeviceController) -> Tuple[Optional[str], str]:
    utils.ui.echo("\n" + "="*61)
    utils.ui.echo(get_string('wf_step2_device_info'))
    utils.ui.echo("="*61)
    
    active_slot_suffix = actions.detect_active_slot_robust(dev)
    
    device_model: Optional[str] = None

    if not dev.skip_adb:
        try:
            utils.ui.echo(get_string("device_get_model_adb"))
            device_model = dev.get_device_model()
            if not device_model:
                utils.ui.echo(get_string("device_err_model_auth"))
                raise RuntimeError(get_string('wf_err_adb_model'))
            else:
                utils.ui.echo(get_string("device_found_model").format(model=device_model))
        except Exception as e:
             raise RuntimeError(get_string('wf_err_get_model').format(e=e))

    active_slot_str = active_slot_suffix if active_slot_suffix else get_string('wf_active_slot_unknown')
    utils.ui.echo(get_string('wf_active_slot').format(slot=active_slot_str))
    utils.ui.echo(get_string('wf_step2_complete'))
    
    return device_model, active_slot_suffix

def _wait_for_input_images() -> None:
    utils.ui.echo(get_string('wf_step3_wait_image'))
    prompt = get_string('wf_step3_prompt')
    utils.wait_for_directory(const.IMAGE_DIR, prompt)
    utils.ui.echo(get_string('wf_step3_found'))

def _convert_images(dev: device.DeviceController, device_model: Optional[str]) -> None:
    step_title_key = 'wf_step4_convert'
    try:
        utils.ui.echo("\n" + "="*61)
        utils.ui.echo(get_string(step_title_key))
        utils.ui.echo("="*61)
        actions.convert_images(dev=dev, device_model=device_model)
        utils.ui.echo(get_string('wf_step4_complete'))
    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
        _handle_step_error(step_title_key, e)

def _decrypt_and_modify_xml(wipe: int) -> None:
    step_title_key = 'wf_step5_modify_xml'
    try:
        utils.ui.echo("\n" + "="*61)
        utils.ui.echo(get_string(step_title_key))
        utils.ui.echo("="*61)
        actions.decrypt_x_files()
        actions.modify_xml(wipe=wipe)
        utils.ui.echo(get_string('wf_step5_complete'))
    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
        _handle_step_error(step_title_key, e)

def _dump_images(dev: device.DeviceController, active_slot_suffix: str) -> Tuple[bool, str, str]:
    step_title_key = 'wf_step6_dump'
    skip_dp_workflow = False
    
    suffix = active_slot_suffix if active_slot_suffix else ""
    boot_target = f"boot{suffix}"
    vbmeta_target = f"vbmeta_system{suffix}"

    try:
        utils.ui.echo("\n" + "="*61)
        utils.ui.echo(get_string(step_title_key))
        utils.ui.echo("="*61)
        
        extra_dumps = [boot_target, vbmeta_target]
        
        utils.ui.echo(get_string('wf_step6_extra_dumps').format(dumps=', '.join(extra_dumps)))
        
        dump_status = actions.read_edl(
            dev=dev,
            skip_reset=False, 
            additional_targets=extra_dumps
        )

        if dump_status == "SKIP_DP":
            skip_dp_workflow = True
            utils.ui.echo(get_string('wf_skip_dp'))
        utils.ui.echo(get_string('wf_step6_complete'))
        
        return skip_dp_workflow, boot_target, vbmeta_target

    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
        _handle_step_error(step_title_key, e)
        return False, "", "" 

def _patch_devinfo(skip_dp_workflow: bool) -> None:
    step_title_key = 'wf_step7_patch_dp'
    try:
        if not skip_dp_workflow:
            utils.ui.echo("\n" + "="*61)
            utils.ui.echo(get_string(step_title_key))
            utils.ui.echo("="*61)
            actions.edit_devinfo_persist()
            utils.ui.echo(get_string('wf_step7_complete'))
        else:
            utils.ui.echo("\n" + "="*61)
            utils.ui.echo(get_string('wf_step7_skipped'))
            utils.ui.echo("="*61)
    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
        _handle_step_error(step_title_key, e)

def _check_and_patch_arb(boot_target: str, vbmeta_target: str) -> None:
    step_title_key = 'wf_step8_check_arb'
    try:
        utils.ui.echo("\n" + "="*61)
        utils.ui.echo(get_string(step_title_key))
        utils.ui.echo("="*61)
        
        utils.ui.echo(get_string('wf_step8_use_dumps'))
        dumped_boot = const.BACKUP_DIR / f"{boot_target}.img"
        dumped_vbmeta = const.BACKUP_DIR / f"{vbmeta_target}.img"
        
        arb_status_result = actions.read_anti_rollback(
            dumped_boot_path=dumped_boot,
            dumped_vbmeta_path=dumped_vbmeta
        )
        
        if arb_status_result[0] == 'ERROR':
            utils.ui.echo("\n" + "!"*61)
            utils.ui.echo(get_string('wf_step8_err_arb_check'))
            utils.ui.echo(get_string('wf_step8_err_arb_check_detail'))
            utils.ui.echo(get_string('wf_step8_err_arb_abort'))
            utils.ui.echo("!"*61)
            raise RuntimeError(get_string('wf_step8_err_arb_abort'))

        actions.patch_anti_rollback(comparison_result=arb_status_result)
        utils.ui.echo(get_string('wf_step8_complete'))
    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
        _handle_step_error(step_title_key, e)

def _flash_images(dev: device.DeviceController, skip_dp_workflow: bool) -> None:
    step_title_key = 'wf_step9_flash'
    try:
        utils.ui.echo("\n" + "="*61)
        utils.ui.echo(get_string(step_title_key))
        utils.ui.echo("="*61)
        utils.ui.echo(get_string('wf_step9_flash_info'))
        actions.flash_edl(dev=dev, skip_reset_edl=True, skip_dp=skip_dp_workflow)
    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
        _handle_step_error(step_title_key, e)

def _handle_step_error(step_title_key: str, e: Exception) -> None:
    utils.ui.echo("\n" + "!" * 61, err=True)
    utils.ui.echo(get_string('wf_err_halted'), err=True)
    utils.ui.echo(f"  [!] {get_string(step_title_key)} - FAILED", err=True)
    utils.ui.echo(get_string('wf_err_details').format(e=e), err=True)
    utils.ui.echo("!" * 61, err=True)
    raise e

def patch_all(dev: device.DeviceController, wipe: int = 0) -> None:
    try:
        _cleanup_previous_outputs(wipe)
        
        device_model, active_slot_suffix = _get_device_info(dev)
        
        _wait_for_input_images()
        
        _convert_images(dev, device_model)
        
        _decrypt_and_modify_xml(wipe)
        
        skip_dp_workflow, boot_target, vbmeta_target = _dump_images(dev, active_slot_suffix)
        
        _patch_devinfo(skip_dp_workflow)
        
        _check_and_patch_arb(boot_target, vbmeta_target)
        
        _flash_images(dev, skip_dp_workflow)
        
        utils.ui.echo("\n" + "=" * 61)
        utils.ui.echo(get_string('wf_process_complete'))
        utils.ui.echo(get_string('wf_process_complete_info'))
        utils.ui.echo(get_string('wf_notice_widevine'))
        utils.ui.echo("=" * 61)

    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
        utils.ui.echo("\n" + "!" * 61)
        utils.ui.echo(get_string('wf_err_halted'))
        utils.ui.echo(get_string('wf_err_details').format(e=e))
        utils.ui.echo("!" * 61)
        raise
    except SystemExit as e:
        utils.ui.echo("\n" + "!" * 61)
        utils.ui.echo(get_string('wf_err_halted_script').format(e=e))
        utils.ui.echo("!" * 61)
        raise
    except KeyboardInterrupt:
        utils.ui.echo("\n" + "!" * 61)
        utils.ui.echo(get_string('wf_err_cancelled'))
        utils.ui.echo("!" * 61)
        raise