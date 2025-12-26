import os
import shutil
import sys
from datetime import datetime
from pathlib import Path
from typing import Optional, List, Dict, Any, Callable, Tuple

from . import edl
from .. import constants as const
from .. import utils, device
from ..partition import ensure_params_or_fail
from ..patch.region import edit_vendor_boot, detect_country_codes, patch_country_codes
from ..patch.avb import extract_image_avb_info, rebuild_vbmeta_with_chained_images
from ..i18n import get_string

def convert_region_images(
    dev: device.DeviceController, 
    device_model: Optional[str] = None,
    on_log: Callable[[str], None] = lambda s: None
) -> None:
    
    on_log(get_string("act_conv_start"))

    on_log(get_string("act_clean_old"))
    if const.OUTPUT_DIR.exists():
        shutil.rmtree(const.OUTPUT_DIR)
    on_log("")

    on_log(get_string("act_wait_vb_vbmeta"))
    const.IMAGE_DIR.mkdir(exist_ok=True)
    
    vendor_boot_src = const.IMAGE_DIR / const.FN_VENDOR_BOOT
    vbmeta_src = const.IMAGE_DIR / const.FN_VBMETA

    if not vendor_boot_src.exists() or not vbmeta_src.exists():
        raise FileNotFoundError(get_string("act_err_xml_missing").format(dir=const.IMAGE_DIR.name))

    on_log(get_string("act_backup_orig"))
    vendor_boot_bak = const.BASE_DIR / const.FN_VENDOR_BOOT_BAK
    vbmeta_bak = const.BASE_DIR / const.FN_VBMETA_BAK
    
    try:
        shutil.copy(vendor_boot_src, vendor_boot_bak)
        shutil.copy(vbmeta_src, vbmeta_bak)
        on_log(get_string("act_backup_complete"))
    except (IOError, OSError) as e:
        raise IOError(get_string("act_err_copy_input").format(e=e))

    on_log(get_string("act_start_conv"))
    edit_vendor_boot(str(vendor_boot_bak))

    vendor_boot_prc = const.BASE_DIR / const.FN_VENDOR_BOOT_PRC
    on_log(get_string("act_verify_conv"))
    if not vendor_boot_prc.exists():
        raise FileNotFoundError(get_string("act_err_vb_prc_not_created"))
    on_log(get_string("act_conv_success"))

    on_log(get_string("act_extract_info"))
    vendor_boot_info = extract_image_avb_info(vendor_boot_bak)
    on_log(get_string("act_info_extracted"))

    if device_model and not dev.skip_adb:
        device_model = device_model.replace(" ", "")
        on_log(get_string("act_val_model").format(model=device_model))
        fingerprint_key = "com.android.build.vendor_boot.fingerprint"
        if fingerprint_key in vendor_boot_info:
            fingerprint = vendor_boot_info[fingerprint_key]
            on_log(get_string("act_found_fp").format(fp=fingerprint))
            if device_model in fingerprint:
                on_log(get_string("act_model_match").format(model=device_model))
            else:
                on_log(get_string("act_model_mismatch").format(model=device_model))
                on_log(get_string("act_rom_mismatch_abort"))
                raise RuntimeError(get_string("act_err_firmware_mismatch"))
        else:
            on_log(get_string("act_warn_fp_missing").format(key=fingerprint_key))
            on_log(get_string("act_skip_val"))
    
    on_log(get_string("act_add_footer_vb"))
    
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
        on_log(get_string("act_restore_props").format(count=len(vendor_boot_info['props_args']) // 2))

    if 'flags' in vendor_boot_info:
        add_hash_footer_cmd.extend(["--flags", vendor_boot_info.get('flags', '0')])
        on_log(get_string("act_restore_flags").format(flags=vendor_boot_info.get('flags', '0')))

    utils.run_command(add_hash_footer_cmd)
    
    vbmeta_img = const.BASE_DIR / const.FN_VBMETA
    rebuild_vbmeta_with_chained_images(
        output_path=vbmeta_img,
        original_vbmeta_path=vbmeta_bak,
        chained_images=[vendor_boot_prc]
    )
    on_log("")

    on_log(get_string("act_finalize"))
    on_log(get_string("act_rename_final"))
    final_vendor_boot = const.BASE_DIR / const.FN_VENDOR_BOOT
    shutil.move(const.BASE_DIR / const.FN_VENDOR_BOOT_PRC, final_vendor_boot)

    final_images = [final_vendor_boot, const.BASE_DIR / const.FN_VBMETA]

    on_log(get_string("act_move_final").format(dir=const.OUTPUT_DIR.name))
    const.OUTPUT_DIR.mkdir(exist_ok=True)
    for img in final_images:
        if img.exists(): 
            shutil.move(img, const.OUTPUT_DIR / img.name)

    on_log(get_string("act_move_backup").format(dir=const.BACKUP_DIR.name))
    const.BACKUP_DIR.mkdir(exist_ok=True)
    for bak_file in const.BASE_DIR.glob("*.bak.img"):
        shutil.move(bak_file, const.BACKUP_DIR / bak_file.name)
    on_log("")

    on_log("  " + "=" * 78)
    on_log(get_string("act_success"))
    on_log(get_string("act_final_saved").format(dir=const.OUTPUT_DIR.name))
    on_log("  " + "=" * 78)

def _default_select_callback(options: List[Tuple[str, str]], prompt_msg: str) -> str:
    print("-" * 60)
    for i, (code, name) in enumerate(options):
        print(f"{i+1:3d}. {name} ({code})")
    print("-" * 60)
    
    while True:
        try:
            choice = input(f"{prompt_msg} ")
            idx = int(choice) - 1
            if 0 <= idx < len(options):
                return options[idx][0]
        except (ValueError, IndexError):
            pass
        print("Invalid selection.")

def edit_devinfo_persist(
    on_log: Callable[[str], None] = lambda s: None,
    on_confirm: Callable[[str], bool] = lambda msg: True,
    on_select: Callable[[List[Tuple[str, str]], str], str] = _default_select_callback
) -> Optional[str]:
    on_log(get_string("act_start_dp_patch"))
    
    on_log(get_string("act_wait_dp"))
    const.BACKUP_DIR.mkdir(exist_ok=True) 

    devinfo_img_src = const.BACKUP_DIR / const.FN_DEVINFO
    persist_img_src = const.BACKUP_DIR / const.FN_PERSIST
    
    devinfo_img = const.BASE_DIR / const.FN_DEVINFO
    persist_img = const.BASE_DIR / const.FN_PERSIST

    if not devinfo_img_src.exists() and not persist_img_src.exists():
         on_log(get_string("act_err_dp_missing_backup"))
         return None

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
    on_log(get_string("act_files_backed_up").format(dir=backup_critical_dir.name))

    on_log(get_string("act_clean_dp_out").format(dir=const.OUTPUT_DP_DIR.name))
    if const.OUTPUT_DP_DIR.exists():
        shutil.rmtree(const.OUTPUT_DP_DIR)
    const.OUTPUT_DP_DIR.mkdir(exist_ok=True)

    on_log(get_string("act_detect_codes"))
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
    
    on_log(get_string("act_detect_result").format(res=', '.join(status_messages)))
    
    if files_found == 0:
        on_log(get_string("act_no_codes_skip"))
        devinfo_img.unlink(missing_ok=True)
        persist_img.unlink(missing_ok=True)
        return backup_critical_dir.name

    on_log(get_string("act_note_region_code"))
    
    should_change = on_confirm(get_string("act_ask_change_code"))

    if not should_change:
        on_log(get_string("act_op_cancel"))
        
        devinfo_img.unlink(missing_ok=True)
        persist_img.unlink(missing_ok=True)
        
        on_log(get_string("act_safety_remove"))
        (const.IMAGE_DIR / const.FN_DEVINFO).unlink(missing_ok=True)
        (const.IMAGE_DIR / const.FN_PERSIST).unlink(missing_ok=True)
        return backup_critical_dir.name

    target_map = detected_codes.copy()
    
    if not const.SORTED_COUNTRY_CODES:
        raise ImportError(get_string("act_err_codes_missing_exc"))
        
    prompt_msg = get_string("act_select_new_code")
    replacement_code = on_select(const.SORTED_COUNTRY_CODES, prompt_msg)
    
    if not replacement_code:
        on_log(get_string("act_select_cancel"))
        return backup_critical_dir.name

    on_log(get_string("act_selected").format(name=replacement_code, code=replacement_code)) # Simplified log
    patch_country_codes(replacement_code, target_map)

    modified_devinfo = const.BASE_DIR / "devinfo_modified.img"
    modified_persist = const.BASE_DIR / "persist_modified.img"
    
    if modified_devinfo.exists():
        shutil.move(modified_devinfo, const.OUTPUT_DP_DIR / const.FN_DEVINFO)
    if modified_persist.exists():
        shutil.move(modified_persist, const.OUTPUT_DP_DIR / const.FN_PERSIST)
        
    on_log(get_string("act_dp_moved").format(dir=const.OUTPUT_DP_DIR.name))
    
    devinfo_img.unlink(missing_ok=True)
    persist_img.unlink(missing_ok=True)
    
    on_log("\n  " + "=" * 78)
    on_log(get_string("act_success"))
    on_log(get_string("act_dp_ready").format(dir=const.OUTPUT_DP_DIR.name))
    on_log("  " + "=" * 78)
    
    return backup_critical_dir.name

def rescue_after_ota(
    dev: device.DeviceController,
    on_log: Callable[[str], None] = lambda s: None
) -> None:
    on_log(get_string("rescue_prompt_files"))

    edl.ensure_edl_requirements()

    on_log(get_string("rescue_wait_adb"))
    dev.wait_for_adb()

    on_log(get_string("rescue_reboot_edl"))
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
            on_log(get_string("rescue_patching_slot").format(slot=slot))
            if not edit_vendor_boot(str(vb_path), copy_if_unchanged=False):
                on_log(get_string("rescue_skip_no_change").format(slot=slot))
                continue
        except Exception as e:
            on_log(get_string("rescue_skip_error").format(slot=slot, e=e))
            continue

        if not prc_temp.exists():
            on_log(get_string("rescue_skip_no_output").format(slot=slot))
            continue

        dest_vb = const.OUTPUT_DIR / f"{vb_target}.img"
        shutil.move(prc_temp, dest_vb)
        patched_map[vb_target] = dest_vb
        
        on_log(get_string("rescue_remaking_vbmeta").format(slot=slot))

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
        on_log(get_string("rescue_nothing_to_flash"))
        return

    on_log(get_string("rescue_wait_adb_flash"))
    
    port = edl._prepare_edl_session(dev)
    
    for target, path in patched_map.items():
        edl.flash_partition_target(dev, port, target, path)
        
    on_log(get_string("act_reset_sys"))
    dev.edl_reset(port)
