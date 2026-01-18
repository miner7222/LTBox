import subprocess
import shutil
from datetime import datetime
from typing import Optional, Tuple

from . import constants as const
from . import utils, device, actions
from .context import TaskContext
from .errors import LTBoxError, UserCancelError, DeviceError
from .i18n import get_string
from .logger import logging_context

def _cleanup_previous_outputs(ctx: TaskContext) -> None:
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
                raise LTBoxError(get_string('utils_remove_error').format(name=folder.name, e=e), e)

def _populate_device_info(ctx: TaskContext) -> None:
    ctx.active_slot_suffix = ctx.dev.detect_active_slot()
    
    if not ctx.dev.skip_adb:
        try:
            ctx.device_model = ctx.dev.adb.get_model()
            if not ctx.device_model:
                raise DeviceError(get_string('wf_err_adb_model'))
        except Exception as e:
             raise DeviceError(get_string('wf_err_get_model').format(e=e), e)

def _wait_for_input_images(ctx: TaskContext) -> None:
    prompt = get_string('act_prompt_image')
    utils.wait_for_directory(const.IMAGE_DIR, prompt)

def _convert_region_images(ctx: TaskContext) -> None:
    actions.convert_region_images(
        dev=ctx.dev, 
        device_model=ctx.device_model,
        target_region=ctx.target_region,
        on_log=ctx.on_log
    )

def _decrypt_and_modify_xml(ctx: TaskContext) -> None:
    actions.decrypt_x_files()
    actions.modify_xml(wipe=ctx.wipe)

def _dump_images(ctx: TaskContext) -> Tuple[bool, str, str]:
    skip_dp_workflow = (ctx.wipe == 0)
    
    suffix = ctx.active_slot_suffix if ctx.active_slot_suffix else ""
    boot_target = f"boot{suffix}"
    vbmeta_target = f"vbmeta_system{suffix}"
    extra_dumps = []
    if not ctx.skip_rollback:
        extra_dumps = [boot_target, vbmeta_target]

    if (not skip_dp_workflow) or extra_dumps:
        actions.dump_partitions(
            dev=ctx.dev,
            skip_reset=False, 
            additional_targets=extra_dumps,
            default_targets=not skip_dp_workflow
        )

    return skip_dp_workflow, boot_target, vbmeta_target

def _patch_devinfo(ctx: TaskContext, skip_dp_workflow: bool) -> Optional[str]:
    if not skip_dp_workflow:
        return actions.edit_devinfo_persist(
            on_log=ctx.on_log,
            on_confirm=lambda msg: utils.ui.prompt(msg + " (y/n) ").lower().strip() == 'y',
            on_select=lambda opts, msg: _select_country_code_adapter(opts, msg)
        )
    return None

def _select_country_code_adapter(options, prompt_msg):
    utils.ui.info(prompt_msg)
    
    count = len(options)
    for i in range(0, count, 2):
        code1, name1 = options[i]
        item1 = f"{i+1:3d}. {name1} ({code1})"
        
        if i + 1 < count:
            code2, name2 = options[i+1]
            item2 = f"{i+2:3d}. {name2} ({code2})"
            utils.ui.info(f"{item1:<40} {item2}")
        else:
            utils.ui.info(item1)
    
    while True:
        choice = utils.ui.prompt(get_string("act_enter_num").format(len=len(options)))
        try:
            idx = int(choice) - 1
            if 0 <= idx < len(options):
                return options[idx][0]
        except ValueError:
            pass
        utils.ui.info(get_string("act_invalid_input"))

def _check_and_patch_arb(ctx: TaskContext, boot_target: str, vbmeta_target: str) -> None:
    dumped_boot = const.BACKUP_DIR / f"{boot_target}.img"
    dumped_vbmeta = const.BACKUP_DIR / f"{vbmeta_target}.img"
    
    arb_status_result = actions.read_anti_rollback(
        dumped_boot_path=dumped_boot,
        dumped_vbmeta_path=dumped_vbmeta
    )
    
    if arb_status_result[0] == 'ERROR':
        raise LTBoxError(get_string('wf_step8_err_arb_abort'))

    actions.patch_anti_rollback(comparison_result=arb_status_result)

def _flash_images(ctx: TaskContext, skip_dp_workflow: bool) -> None:
    actions.flash_full_firmware(dev=ctx.dev, skip_reset_edl=True, skip_dp=skip_dp_workflow)

def patch_all(dev: device.DeviceController, wipe: int = 0, skip_rollback: bool = False, target_region: str = "PRC") -> str:
    ctx = TaskContext(
        dev=dev, 
        wipe=wipe, 
        skip_rollback=skip_rollback,
        target_region=target_region,
        on_log=lambda s: utils.ui.info(s)
    )

    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    log_file = f"log_{timestamp}.txt"
    command_name = "patch_all_wipe" if wipe == 1 else "patch_all"

    utils.ui.info(get_string("logging_enabled").format(log_file=log_file))
    utils.ui.info(get_string("logging_command").format(command=command_name))
    
    if target_region == "ROW":
        utils.ui.info(get_string("menu_main_install_keep_row"))
    else:
        utils.ui.info(get_string("menu_main_install_keep_prc"))

    try:
        with logging_context(log_file):
            ctx.on_log(get_string('wf_step1_clean'))
            if ctx.wipe == 1:
                ctx.on_log(get_string('wf_wipe_mode_start'))
            else:
                ctx.on_log(get_string('wf_nowipe_mode_start'))
            _cleanup_previous_outputs(ctx)
            
            ctx.on_log(get_string('wf_step2_device_info'))
            _populate_device_info(ctx)
            
            active_slot_str = ctx.active_slot_suffix if ctx.active_slot_suffix else get_string('wf_active_slot_unknown')
            ctx.on_log(get_string('act_active_slot').format(slot=active_slot_str))
            
            ctx.on_log(get_string('wf_step3_wait_image'))
            _wait_for_input_images(ctx)
            ctx.on_log(get_string('wf_step3_found'))
            
            ctx.on_log(get_string('wf_step4_convert'))
            _convert_region_images(ctx)
            
            ctx.on_log(get_string('wf_step5_modify_xml'))
            _decrypt_and_modify_xml(ctx)
            
            ctx.on_log(get_string('wf_step6_dump'))
            skip_dp_workflow, boot_target, vbmeta_target = _dump_images(ctx)
            
            if skip_dp_workflow:
                ctx.on_log(get_string('wf_step7_skipped'))
            else:
                ctx.on_log(get_string('wf_step7_patch_dp'))
            backup_dir_name = _patch_devinfo(ctx, skip_dp_workflow)
            
            if not ctx.skip_rollback:
                ctx.on_log(get_string('wf_step8_check_arb'))
                _check_and_patch_arb(ctx, boot_target, vbmeta_target)
            else:
                ctx.on_log(get_string('wf_step8_skipped'))
            
            ctx.on_log(get_string('wf_step9_flash'))
            _flash_images(ctx, skip_dp_workflow)
            
            success_msg = get_string('wf_process_complete')
            success_msg += f"\n{get_string('wf_process_complete_info')}"
            
            if backup_dir_name:
                success_msg += f"\n\n{get_string('wf_backup_notice').format(dir=backup_dir_name)}"

            success_msg += f"\n\n{get_string('wf_notice_widevine')}"
            return success_msg

    except LTBoxError as e:
        utils.ui.echo(get_string('wf_err_halted'), err=True)
        raise e
    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
        utils.ui.echo(get_string('wf_err_halted'), err=True)
        raise e
    except SystemExit as e:
        raise LTBoxError(get_string('wf_err_halted_script').format(e=e), e)
    except KeyboardInterrupt:
        raise UserCancelError(get_string('act_op_cancel'))
    finally:
        utils.ui.info(get_string("logging_finished").format(log_file=log_file))