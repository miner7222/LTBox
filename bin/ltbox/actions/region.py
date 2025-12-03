import os
import shutil
import sys
from datetime import datetime
from pathlib import Path
from typing import Optional, List, Dict, Any

from . import edl
from .. import constants as const
from .. import utils, device
from ..partition import ensure_params_or_fail
from ..patch.region import edit_vendor_boot, detect_country_codes, patch_country_codes
from ..patch.avb import extract_image_avb_info, rebuild_vbmeta_with_chained_images
from ..i18n import get_string

def convert_region_images(dev: device.DeviceController, device_model: Optional[str] = None) -> None:
    utils.check_dependencies()
    
    utils.ui.info(get_string("act_conv_start"))

    utils.ui.info(get_string("act_clean_old"))
    if const.OUTPUT_DIR.exists():
        shutil.rmtree(const.OUTPUT_DIR)
    utils.ui.info("")

    utils.ui.info(get_string("act_wait_vb_vbmeta"))
    const.IMAGE_DIR.mkdir(exist_ok=True)
    required_files = [const.FN_VENDOR_BOOT, const.FN_VBMETA]
    prompt = get_string("act_prompt_vb_vbmeta").format(name=const.IMAGE_DIR.name)
    utils.wait_for_files(const.IMAGE_DIR, required_files, prompt)
    
    vendor_boot_src = const.IMAGE_DIR / const.FN_VENDOR_BOOT
    vbmeta_src = const.IMAGE_DIR / const.FN_VBMETA

    utils.ui.info(get_string("act_backup_orig"))
    vendor_boot_bak = const.BASE_DIR / const.FN_VENDOR_BOOT_BAK
    vbmeta_bak = const.BASE_DIR / const.FN_VBMETA_BAK
    
    try:
        shutil.copy(vendor_boot_src, vendor_boot_bak)
        shutil.copy(vbmeta_src, vbmeta_bak)
        utils.ui.info(get_string("act_backup_complete"))
    except (IOError, OSError) as e:
        utils.ui.error(get_string("act_err_copy_input").format(e=e))
        raise

    utils.ui.info(get_string("act_start_conv"))
    edit_vendor_boot(str(vendor_boot_bak))

    vendor_boot_prc = const.BASE_DIR / const.FN_VENDOR_BOOT_PRC
    utils.ui.info(get_string("act_verify_conv"))
    if not vendor_boot_prc.exists():
        utils.ui.error(get_string("act_err_vb_prc_missing"))
        raise FileNotFoundError(get_string("act_err_vb_prc_not_created"))
    utils.ui.info(get_string("act_conv_success"))

    utils.ui.info(get_string("act_extract_info"))
    vendor_boot_info = extract_image_avb_info(vendor_boot_bak)
    utils.ui.info(get_string("act_info_extracted"))

    if device_model and not dev.skip_adb:
        utils.ui.info(get_string("act_val_model").format(model=device_model))
        fingerprint_key = "com.android.build.vendor_boot.fingerprint"
        if fingerprint_key in vendor_boot_info:
            fingerprint = vendor_boot_info[fingerprint_key]
            utils.ui.info(get_string("act_found_fp").format(fp=fingerprint))
            if device_model in fingerprint:
                utils.ui.info(get_string("act_model_match").format(model=device_model))
            else:
                utils.ui.info(get_string("act_model_mismatch").format(model=device_model))
                utils.ui.info(get_string("act_rom_mismatch_abort"))
                raise RuntimeError(get_string("act_err_firmware_mismatch"))
        else:
            utils.ui.info(get_string("act_warn_fp_missing").format(key=fingerprint_key))
            utils.ui.info(get_string("act_skip_val"))
    
    utils.ui.info(get_string("act_add_footer_vb"))
    
    for key in ['partition_size', 'name', 'rollback', 'salt']:
        if key not in vendor_boot_info:
            if key == 'partition_size' and 'data_size' in vendor_boot_info:
                 vendor_boot_info['partition_size'] = vendor_boot_info['data_size']
            else:
                raise KeyError(get_string("act_err_avb_key_missing").format(key=key, name=vendor_boot_bak.name))

    add_hash_footer_cmd = [
        str(const.PYTHON_EXE), str(const.AVBTOOL_PY), "add_hash_footer",
        "--image", str(vendor_boot_prc),
        "--partition_size", vendor_boot_info['partition_size'],
        "--partition_name", vendor_boot_info['name'],
        "--rollback_index", vendor_boot_info['rollback'],
        "--salt", vendor_boot_info['salt']
    ]
    
    if 'props_args' in vendor_boot_info:
        add_hash_footer_cmd.extend(vendor_boot_info['props_args'])
        utils.ui.info(get_string("act_restore_props").format(count=len(vendor_boot_info['props_args']) // 2))

    if 'flags' in vendor_boot_info:
        add_hash_footer_cmd.extend(["--flags", vendor_boot_info.get('flags', '0')])
        utils.ui.info(get_string("act_restore_flags").format(flags=vendor_boot_info.get('flags', '0')))

    utils.run_command(add_hash_footer_cmd)
    
    vbmeta_img = const.BASE_DIR / const.FN_VBMETA
    rebuild_vbmeta_with_chained_images(
        output_path=vbmeta_img,
        original_vbmeta_path=vbmeta_bak,
        chained_images=[vendor_boot_prc]
    )
    utils.ui.info("")

    utils.ui.info(get_string("act_finalize"))
    utils.ui.info(get_string("act_rename_final"))
    final_vendor_boot = const.BASE_DIR / const.FN_VENDOR_BOOT
    shutil.move(const.BASE_DIR / const.FN_VENDOR_BOOT_PRC, final_vendor_boot)

    final_images = [final_vendor_boot, const.BASE_DIR / const.FN_VBMETA]

    utils.ui.info(get_string("act_move_final").format(dir=const.OUTPUT_DIR.name))
    const.OUTPUT_DIR.mkdir(exist_ok=True)
    for img in final_images:
        if img.exists(): 
            shutil.move(img, const.OUTPUT_DIR / img.name)

    utils.ui.info(get_string("act_move_backup").format(dir=const.BACKUP_DIR.name))
    const.BACKUP_DIR.mkdir(exist_ok=True)
    for bak_file in const.BASE_DIR.glob("*.bak.img"):
        shutil.move(bak_file, const.BACKUP_DIR / bak_file.name)
    utils.ui.info("")

    utils.ui.info("  " + "=" * 78)
    utils.ui.info(get_string("act_success"))
    utils.ui.info(get_string("act_final_saved").format(dir=const.OUTPUT_DIR.name))
    utils.ui.info("  " + "=" * 78)

def select_country_code(prompt_message: str = "Please select a country from the list below:") -> str:
    utils.ui.info(get_string("act_prompt_msg").format(msg=prompt_message.upper()))

    if not const.SORTED_COUNTRY_CODES:
        utils.ui.error(get_string("act_err_codes_missing"))
        raise ImportError(get_string("act_err_codes_missing_exc"))

    sorted_countries = const.SORTED_COUNTRY_CODES
    
    num_cols = 2
    col_width = 38 
    
    line_width = col_width * num_cols
    utils.ui.info("-" * line_width)
    
    for i in range(0, len(sorted_countries), num_cols):
        line = []
        for j in range(num_cols):
            idx = i + j
            if idx < len(sorted_countries):
                code, name = sorted_countries[idx]
                line.append(f"{idx+1:3d}. {name} ({code})".ljust(col_width))
        utils.ui.info("".join(line))
    utils.ui.info("-" * line_width)

    while True:
        try:
            choice = utils.ui.prompt(get_string("act_enter_num").format(len=len(sorted_countries)))
            choice_idx = int(choice) - 1
            if 0 <= choice_idx < len(sorted_countries):
                selected_code = sorted_countries[choice_idx][0]
                selected_name = sorted_countries[choice_idx][1]
                utils.ui.info(get_string("act_selected").format(name=selected_name, code=selected_code))
                return selected_code
            else:
                utils.ui.info(get_string("act_invalid_num"))
        except ValueError:
            utils.ui.info(get_string("act_invalid_input"))
        except (KeyboardInterrupt, EOFError):
            utils.ui.info(get_string("act_select_cancel"))
            raise KeyboardInterrupt(get_string("act_select_cancel"))

def edit_devinfo_persist() -> Optional[str]:
    utils.ui.info(get_string("act_start_dp_patch"))
    
    utils.ui.info(get_string("act_wait_dp"))
    const.BACKUP_DIR.mkdir(exist_ok=True) 

    devinfo_img_src = const.BACKUP_DIR / const.FN_DEVINFO
    persist_img_src = const.BACKUP_DIR / const.FN_PERSIST
    
    devinfo_img = const.BASE_DIR / const.FN_DEVINFO
    persist_img = const.BASE_DIR / const.FN_PERSIST

    if not devinfo_img_src.exists() and not persist_img_src.exists():
        prompt = get_string("act_prompt_dp").format(dir=const.BACKUP_DIR.name)
        while not devinfo_img_src.exists() and not persist_img_src.exists():
            utils.ui.clear()
            utils.ui.info(get_string("act_wait_files_title"))
            utils.ui.info(prompt)
            utils.ui.info(get_string("act_place_one_file").format(dir=const.BACKUP_DIR.name))
            utils.ui.info(get_string("act_dp_list_item").format(filename=const.FN_DEVINFO))
            utils.ui.info(get_string("act_dp_list_item").format(filename=const.FN_PERSIST))
            utils.ui.info(get_string("press_enter_to_continue"))
            try:
                utils.ui.prompt()
            except EOFError:
                raise RuntimeError(get_string('act_op_cancel'))

    if devinfo_img_src.exists():
        shutil.copy(devinfo_img_src, devinfo_img)
    if persist_img_src.exists():
        shutil.copy(persist_img_src, persist_img)

    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    backup_critical_dir = const.BASE_DIR / f"backup_critical_{timestamp}"
    backup_critical_dir.mkdir(exist_ok=True)
    
    if devinfo_img.exists():
        shutil.copy(devinfo_img, backup_critical_dir)
    if persist_img.exists():
        shutil.copy(persist_img, backup_critical_dir)
    utils.ui.info(get_string("act_files_backed_up").format(dir=backup_critical_dir.name))

    utils.ui.info(get_string("act_clean_dp_out").format(dir=const.OUTPUT_DP_DIR.name))
    if const.OUTPUT_DP_DIR.exists():
        shutil.rmtree(const.OUTPUT_DP_DIR)
    const.OUTPUT_DP_DIR.mkdir(exist_ok=True)

    utils.ui.info(get_string("act_detect_codes"))
    detected_codes = detect_country_codes()
    
    status_messages = []
    files_found = 0
    
    display_order = [const.FN_PERSIST, const.FN_DEVINFO]
    
    for fname in display_order:
        if fname in detected_codes:
            code = detected_codes[fname]
            display_name = Path(fname).stem 
            
            if code:
                status_messages.append(get_string("act_detect_status_found").format(display_name=display_name, code=code))
                files_found += 1
            else:
                status_messages.append(get_string("act_detect_status_null").format(display_name=display_name))
    
    utils.ui.info(get_string("act_detect_result").format(res=', '.join(status_messages)))
    
    if files_found == 0:
        utils.ui.info(get_string("act_no_codes_skip"))
        devinfo_img.unlink(missing_ok=True)
        persist_img.unlink(missing_ok=True)
        return backup_critical_dir.name

    utils.ui.info(get_string("act_note_region_code"))
    utils.ui.info(get_string("act_ask_change_code"))
    choice = ""
    while choice not in ['y', 'n']:
        choice = utils.ui.prompt(get_string("act_enter_yn")).lower().strip()

    if choice == 'n':
        utils.ui.info(get_string("act_op_cancel"))
        
        devinfo_img.unlink(missing_ok=True)
        persist_img.unlink(missing_ok=True)
        
        utils.ui.info(get_string("act_safety_remove"))
        (const.IMAGE_DIR / const.FN_DEVINFO).unlink(missing_ok=True)
        (const.IMAGE_DIR / const.FN_PERSIST).unlink(missing_ok=True)
        return backup_critical_dir.name

    if choice == 'y':
        target_map = detected_codes.copy()
        replacement_code = select_country_code(get_string("act_select_new_code"))
        patch_country_codes(replacement_code, target_map)

        modified_devinfo = const.BASE_DIR / "devinfo_modified.img"
        modified_persist = const.BASE_DIR / "persist_modified.img"
        
        if modified_devinfo.exists():
            shutil.move(modified_devinfo, const.OUTPUT_DP_DIR / const.FN_DEVINFO)
        if modified_persist.exists():
            shutil.move(modified_persist, const.OUTPUT_DP_DIR / const.FN_PERSIST)
            
        utils.ui.info(get_string("act_dp_moved").format(dir=const.OUTPUT_DP_DIR.name))
        
        devinfo_img.unlink(missing_ok=True)
        persist_img.unlink(missing_ok=True)
        
        utils.ui.info("\n  " + "=" * 78)
        utils.ui.info(get_string("act_success"))
        utils.ui.info(get_string("act_dp_ready").format(dir=const.OUTPUT_DP_DIR.name))
        utils.ui.info("  " + "=" * 78)
    
    return backup_critical_dir.name

def rescue_after_ota(dev: device.DeviceController) -> None:
    utils.ui.echo(get_string("rescue_prompt_files"))

    edl.ensure_edl_requirements()

    utils.ui.echo(get_string("rescue_wait_adb"))
    dev.wait_for_adb()

    utils.ui.echo(get_string("rescue_reboot_edl"))
    dev.reboot_to_edl()
    
    slots = ['a', 'b']
    targets = [f'vendor_boot_{s}' for s in slots] + [f'vbmeta_{s}' for s in slots]

    edl.dump_partitions(dev, skip_reset=False, additional_targets=targets, default_targets=False)
    
    const.OUTPUT_DIR.mkdir(exist_ok=True)
    patched_map = {}

    for slot in slots:
        vb_target = f'vendor_boot_{slot}'
        vbmeta_target = f'vbmeta_{slot}'
        
        vb_path = const.BACKUP_DIR / f"{vb_target}.img"
        vbmeta_path = const.BACKUP_DIR / f"{vbmeta_target}.img"
        
        if not vb_path.exists() or not vbmeta_path.exists():
            continue

        prc_temp = vb_path.parent / const.FN_VENDOR_BOOT_PRC
        prc_temp.unlink(missing_ok=True)

        try:
            utils.ui.echo(get_string("rescue_patching_slot").format(slot=slot))
            if not edit_vendor_boot(str(vb_path), copy_if_unchanged=False):
                utils.ui.echo(get_string("rescue_skip_no_change").format(slot=slot))
                continue
        except Exception as e:
            utils.ui.echo(get_string("rescue_skip_error").format(slot=slot, e=e))
            continue

        if not prc_temp.exists():
            utils.ui.echo(get_string("rescue_skip_no_output").format(slot=slot))
            continue

        dest_vb = const.OUTPUT_DIR / f"{vb_target}.img"
        shutil.move(prc_temp, dest_vb)
        patched_map[vb_target] = dest_vb
        
        utils.ui.echo(get_string("rescue_remaking_vbmeta").format(slot=slot))

        vb_info = extract_image_avb_info(vb_path)
        part_size = vb_info.get('partition_size', vb_info.get('data_size'))
        
        cmd_footer = [
            str(const.PYTHON_EXE), str(const.AVBTOOL_PY), "add_hash_footer",
            "--image", str(dest_vb),
            "--partition_size", part_size,
            "--partition_name", "vendor_boot",
            "--rollback_index", vb_info.get('rollback', '0'),
            "--salt", vb_info.get('salt', '')
        ]
        if 'props_args' in vb_info: cmd_footer.extend(vb_info['props_args'])
        if 'flags' in vb_info: cmd_footer.extend(["--flags", vb_info['flags']])
        utils.run_command(cmd_footer)
        
        dest_vbmeta = const.OUTPUT_DIR / f"{vbmeta_target}.img"
        
        rebuild_vbmeta_with_chained_images(
            output_path=dest_vbmeta,
            original_vbmeta_path=vbmeta_path,
            chained_images=[dest_vb]
        )

        patched_map[vbmeta_target] = dest_vbmeta

    if not patched_map:
        utils.ui.echo(get_string("rescue_nothing_to_flash"))
        return

    utils.ui.echo(get_string("rescue_wait_adb_flash"))
    
    port = edl._prepare_edl_session(dev)
    
    for target, path in patched_map.items():
        edl.flash_partition_target(dev, port, target, path)
        
    utils.ui.echo(get_string("act_reset_sys"))
    dev.edl_reset(port)