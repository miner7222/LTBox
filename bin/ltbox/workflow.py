import subprocess
import shutil
from typing import Optional, Tuple

from . import constants as const
from . import utils, device, actions
from .i18n import get_string
from .errors import ToolError

def _cleanup_previous_outputs(wipe: int) -> None:
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
            except OSError as e:
                raise ToolError(get_string('wf_remove_error').format(name=folder.name, e=e))

def _get_device_info(dev: device.DeviceController) -> Tuple[Optional[str], str]:
    active_slot_suffix = actions.detect_active_slot_robust(dev)
    
    device_model: Optional[str] = None

    if not dev.skip_adb:
        try:
            device_model = dev.get_device_model()
            if not device_model:
                raise ToolError(get_string('wf_err_adb_model'))
        except Exception as e:
             raise ToolError(get_string('wf_err_get_model').format(e=e))
    
    return device_model, active_slot_suffix

def _wait_for_input_images() -> None:
    prompt = get_string('wf_step3_prompt')
    utils.wait_for_directory(const.IMAGE_DIR, prompt)

def _convert_images(dev: device.DeviceController, device_model: Optional[str]) -> None:
    actions.convert_images(dev=dev, device_model=device_model)

def _decrypt_and_modify_xml(wipe: int) -> None:
    actions.decrypt_x_files()
    actions.modify_xml(wipe=wipe)

def _dump_images(dev: device.DeviceController, active_slot_suffix: str) -> Tuple[bool, str, str]:
    skip_dp_workflow = False
    
    suffix = active_slot_suffix if active_slot_suffix else ""
    boot_target = f"boot{suffix}"
    vbmeta_target = f"vbmeta_system{suffix}"
    
    extra_dumps = [boot_target, vbmeta_target]
        
    actions.read_edl(
        dev=dev,
        skip_reset=False, 
        additional_targets=extra_dumps
    )

    return skip_dp_workflow, boot_target, vbmeta_target

def _patch_devinfo(skip_dp_workflow: bool) -> Optional[str]:
    if not skip_dp_workflow:
        return actions.edit_devinfo_persist()
    return None

def _check_and_patch_arb(boot_target: str, vbmeta_target: str) -> None:
    dumped_boot = const.BACKUP_DIR / f"{boot_target}.img"
    dumped_vbmeta = const.BACKUP_DIR / f"{vbmeta_target}.img"
    
    arb_status_result = actions.read_anti_rollback(
        dumped_boot_path=dumped_boot,
        dumped_vbmeta_path=dumped_vbmeta
    )
    
    if arb_status_result[0] == 'ERROR':
        raise ToolError(get_string('wf_step8_err_arb_abort'))

    actions.patch_anti_rollback(comparison_result=arb_status_result)

def _flash_images(dev: device.DeviceController, skip_dp_workflow: bool) -> None:
    actions.flash_edl(dev=dev, skip_reset_edl=True, skip_dp=skip_dp_workflow)

def _handle_step_error(step_title_key: str, e: Exception) -> None:
    utils.ui.echo("\n" + "!" * 61, err=True)
    utils.ui.echo(get_string('wf_err_halted'), err=True)
    utils.ui.echo(get_string('wf_err_step_failed').format(title=get_string(step_title_key)), err=True)
    utils.ui.echo(get_string('wf_err_details').format(e=e), err=True)
    utils.ui.echo("!" * 61, err=True)
    raise e

def patch_all(dev: device.DeviceController, wipe: int = 0) -> str:
    try:
        utils.ui.echo(get_string('wf_step1_clean'))
        if wipe == 1:
            utils.ui.echo(get_string('wf_wipe_mode_start'))
        else:
            utils.ui.echo(get_string('wf_nowipe_mode_start'))
        _cleanup_previous_outputs(wipe)
        
        utils.ui.echo(get_string('wf_step2_device_info'))
        device_model, active_slot_suffix = _get_device_info(dev)
        active_slot_str = active_slot_suffix if active_slot_suffix else get_string('wf_active_slot_unknown')
        utils.ui.echo(get_string('wf_active_slot').format(slot=active_slot_str))
        
        utils.ui.echo(get_string('wf_step3_wait_image'))
        _wait_for_input_images()
        utils.ui.echo(get_string('wf_step3_found'))
        
        utils.ui.echo(get_string('wf_step4_convert'))
        _convert_images(dev, device_model)
        
        utils.ui.echo(get_string('wf_step5_modify_xml'))
        _decrypt_and_modify_xml(wipe)
        
        utils.ui.echo(get_string('wf_step6_dump'))
        skip_dp_workflow, boot_target, vbmeta_target = _dump_images(dev, active_slot_suffix)
        
        utils.ui.echo(get_string('wf_step7_patch_dp'))
        backup_dir_name = _patch_devinfo(skip_dp_workflow)
        
        utils.ui.echo(get_string('wf_step8_check_arb'))
        _check_and_patch_arb(boot_target, vbmeta_target)
        
        utils.ui.echo(get_string('wf_step9_flash'))
        _flash_images(dev, skip_dp_workflow)
        
        success_msg = get_string('wf_process_complete')
        success_msg += f"\n{get_string('wf_process_complete_info')}"
        
        if backup_dir_name:
            success_msg += f"\n\n{get_string('wf_backup_notice').format(dir=backup_dir_name)}"

        success_msg += f"\n\n{get_string('wf_notice_widevine')}"
        return success_msg

    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError, ToolError) as e:
        utils.ui.echo(get_string('wf_err_halted'), err=True)
        raise e
    except SystemExit as e:
        raise ToolError(get_string('wf_err_halted_script').format(e=e))
    except KeyboardInterrupt:
        raise ToolError(get_string('process_cancelled'))