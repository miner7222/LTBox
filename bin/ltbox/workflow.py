import subprocess
import sys
import shutil
from typing import Optional, Dict

from . import constants as const
from . import utils, device, actions
from .i18n import get_string

def patch_all(dev: device.DeviceController, wipe: int = 0) -> None:
    
    print(get_string('wf_step1_clean'))
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
                print(get_string('wf_removed').format(name=folder.name))
            except OSError as e:
                print(get_string('wf_remove_error').format(name=folder.name, e=e), file=sys.stderr)

    if wipe == 1:
        print(get_string('wf_wipe_mode_start'))
    else:
        print(get_string('wf_nowipe_mode_start'))
    
    print("\n" + "="*61)
    print(get_string('wf_step2_device_info'))
    print("="*61)
    
    active_slot_suffix = actions.detect_active_slot_robust(dev)
    
    device_model: Optional[str] = None

    if not dev.skip_adb:
        try:
            print(get_string("device_get_model_adb"))
            device_model = dev.get_device_model()
            if not device_model:
                print(get_string("device_err_model_auth"))
                raise RuntimeError(get_string('wf_err_adb_model'))
            else:
                print(get_string("device_found_model").format(model=device_model))
        except Exception as e:
             raise RuntimeError(get_string('wf_err_get_model').format(e=e))

    active_slot_str = active_slot_suffix if active_slot_suffix else get_string('wf_active_slot_unknown')
    print(get_string('wf_active_slot').format(slot=active_slot_str))
    print(get_string('wf_step2_complete'))

    print(get_string('wf_step3_wait_image'))
    prompt = get_string('wf_step3_prompt')
    utils.wait_for_directory(const.IMAGE_DIR, prompt)
    print(get_string('wf_step3_found'))
    
    skip_dp_workflow = False
    
    try:
        step_title_key = 'wf_step4_convert'
        try:
            print("\n" + "="*61)
            print(get_string(step_title_key))
            print("="*61)
            actions.convert_images(dev=dev, device_model=device_model)
            print(get_string('wf_step4_complete'))
        except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
            print("\n" + "!" * 61, file=sys.stderr)
            print(get_string('wf_err_halted'), file=sys.stderr)
            print(f"  [!] {get_string(step_title_key)} - FAILED", file=sys.stderr)
            print(get_string('wf_err_details').format(e=e), file=sys.stderr)
            print("!" * 61, file=sys.stderr)
            raise

        step_title_key = 'wf_step5_modify_xml'
        try:
            print("\n" + "="*61)
            print(get_string(step_title_key))
            print("="*61)
            actions.decrypt_x_files()
            actions.modify_xml(wipe=wipe)
            print(get_string('wf_step5_complete'))
        except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
            print("\n" + "!" * 61, file=sys.stderr)
            print(get_string('wf_err_halted'), file=sys.stderr)
            print(f"  [!] {get_string(step_title_key)} - FAILED", file=sys.stderr)
            print(get_string('wf_err_details').format(e=e), file=sys.stderr)
            print("!" * 61, file=sys.stderr)
            raise
        
        step_title_key = 'wf_step6_dump'
        try:
            print("\n" + "="*61)
            print(get_string(step_title_key))
            print("="*61)

            suffix = active_slot_suffix if active_slot_suffix else ""
            boot_target = f"boot{suffix}"
            vbmeta_target = f"vbmeta_system{suffix}"
            
            extra_dumps = [boot_target, vbmeta_target]
            
            print(get_string('wf_step6_extra_dumps').format(dumps=', '.join(extra_dumps)))
            
            dump_status = actions.read_edl(
                dev=dev,
                skip_reset=False, 
                additional_targets=extra_dumps
            )

            if dump_status == "SKIP_DP":
                skip_dp_workflow = True
                print(get_string('wf_skip_dp'))
            print(get_string('wf_step6_complete'))
        except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
            print("\n" + "!" * 61, file=sys.stderr)
            print(get_string('wf_err_halted'), file=sys.stderr)
            print(f"  [!] {get_string(step_title_key)} - FAILED", file=sys.stderr)
            print(get_string('wf_err_details').format(e=e), file=sys.stderr)
            print("!" * 61, file=sys.stderr)
            raise
        
        
        step_title_key = 'wf_step7_patch_dp'
        try:
            if not skip_dp_workflow:
                print("\n" + "="*61)
                print(get_string(step_title_key))
                print("="*61)
                actions.edit_devinfo_persist()
                print(get_string('wf_step7_complete'))
            else:
                print("\n" + "="*61)
                print(get_string('wf_step7_skipped'))
                print("="*61)
        except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
            print("\n" + "!" * 61, file=sys.stderr)
            print(get_string('wf_err_halted'), file=sys.stderr)
            print(f"  [!] {get_string(step_title_key)} - FAILED", file=sys.stderr)
            print(get_string('wf_err_details').format(e=e), file=sys.stderr)
            print("!" * 61, file=sys.stderr)
            raise

        
        step_title_key = 'wf_step8_check_arb'
        try:
            print("\n" + "="*61)
            print(get_string(step_title_key))
            print("="*61)
            
            print(get_string('wf_step8_use_dumps'))
            dumped_boot = const.BACKUP_DIR / f"{boot_target}.img"
            dumped_vbmeta = const.BACKUP_DIR / f"{vbmeta_target}.img"
            
            arb_status_result = actions.read_anti_rollback(
                dumped_boot_path=dumped_boot,
                dumped_vbmeta_path=dumped_vbmeta
            )
            
            if arb_status_result[0] == 'ERROR':
                print("\n" + "!"*61)
                print(get_string('wf_step8_err_arb_check'))
                print(get_string('wf_step8_err_arb_check_detail'))
                print(get_string('wf_step8_err_arb_abort'))
                print("!"*61)
                raise RuntimeError(get_string('wf_step8_err_arb_abort'))

            actions.patch_anti_rollback(comparison_result=arb_status_result)
            print(get_string('wf_step8_complete'))
        except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
            print("\n" + "!" * 61, file=sys.stderr)
            print(get_string('wf_err_halted'), file=sys.stderr)
            print(f"  [!] {get_string(step_title_key)} - FAILED", file=sys.stderr)
            print(get_string('wf_err_details').format(e=e), file=sys.stderr)
            print("!" * 61, file=sys.stderr)
            raise
        
        step_title_key = 'wf_step9_flash'
        try:
            print("\n" + "="*61)
            print(get_string(step_title_key))
            print("="*61)
            print(get_string('wf_step9_flash_info'))
            actions.flash_edl(dev=dev, skip_reset_edl=True, skip_dp=skip_dp_workflow)
        except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
            print("\n" + "!" * 61, file=sys.stderr)
            print(get_string('wf_err_halted'), file=sys.stderr)
            print(f"  [!] {get_string(step_title_key)} - FAILED", file=sys.stderr)
            print(get_string('wf_err_details').format(e=e), file=sys.stderr)
            print("!" * 61, file=sys.stderr)
            raise
        
        print("\n" + "=" * 61)
        print(get_string('wf_process_complete'))
        print(get_string('wf_process_complete_info'))
        print(get_string('wf_notice_widevine'))
        print("=" * 61)

    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
        print("\n" + "!" * 61)
        print(get_string('wf_err_halted'))
        print(get_string('wf_err_details').format(e=e))
        print("!" * 61)
        raise
    except SystemExit as e:
        print("\n" + "!" * 61)
        print(get_string('wf_err_halted_script').format(e=e))
        print("!" * 61)
        raise
    except KeyboardInterrupt:
        print("\n" + "!" * 61)
        print(get_string('wf_err_cancelled'))
        print("!" * 61)
        raise