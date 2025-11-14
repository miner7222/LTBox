import os
import platform
import shutil
import sys
from datetime import datetime
from pathlib import Path
from typing import Optional, List, Dict, Any

from .. import constants as const
from .. import utils, device
from ..patch.region import edit_vendor_boot, detect_region_codes, patch_region_codes
from ..patch.avb import extract_image_avb_info, _apply_hash_footer
from ..i18n import get_string

def convert_images(dev: device.DeviceController, device_model: Optional[str] = None) -> None:
    utils.check_dependencies()
    
    print(get_string("act_conv_start"))

    print(get_string("act_clean_old"))
    if const.OUTPUT_DIR.exists():
        shutil.rmtree(const.OUTPUT_DIR)
    print()

    print(get_string("act_wait_vb_vbmeta"))
    const.IMAGE_DIR.mkdir(exist_ok=True)
    required_files = ["vendor_boot.img", "vbmeta.img"]
    prompt = get_string("act_prompt_vb_vbmeta").format(name=const.IMAGE_DIR.name)
    utils.wait_for_files(const.IMAGE_DIR, required_files, prompt)
    
    vendor_boot_src = const.IMAGE_DIR / "vendor_boot.img"
    vbmeta_src = const.IMAGE_DIR / "vbmeta.img"

    print(get_string("act_backup_orig"))
    vendor_boot_bak = const.BASE_DIR / "vendor_boot.bak.img"
    vbmeta_bak = const.BASE_DIR / "vbmeta.bak.img"
    
    try:
        shutil.copy(vendor_boot_src, vendor_boot_bak)
        shutil.copy(vbmeta_src, vbmeta_bak)
        print(get_string("act_backup_complete"))
    except (IOError, OSError) as e:
        print(get_string("act_err_copy_input").format(e=e), file=sys.stderr)
        raise

    print(get_string("act_start_conv"))
    edit_vendor_boot(str(vendor_boot_bak))

    vendor_boot_prc = const.BASE_DIR / "vendor_boot_prc.img"
    print(get_string("act_verify_conv"))
    if not vendor_boot_prc.exists():
        print(get_string("act_err_vb_prc_missing"))
        raise FileNotFoundError(get_string("act_err_vb_prc_not_created"))
    print(get_string("act_conv_success"))

    print(get_string("act_extract_info"))
    vbmeta_info = extract_image_avb_info(vbmeta_bak)
    vendor_boot_info = extract_image_avb_info(vendor_boot_bak)
    print(get_string("act_info_extracted"))

    if device_model and not dev.skip_adb:
        print(get_string("act_val_model").format(model=device_model))
        fingerprint_key = "com.android.build.vendor_boot.fingerprint"
        if fingerprint_key in vendor_boot_info:
            fingerprint = vendor_boot_info[fingerprint_key]
            print(get_string("act_found_fp").format(fp=fingerprint))
            if device_model in fingerprint:
                print(get_string("act_model_match").format(model=device_model))
            else:
                print(get_string("act_model_mismatch").format(model=device_model))
                print(get_string("act_rom_mismatch_abort"))
                raise RuntimeError(get_string("act_err_firmware_mismatch"))
        else:
            print(get_string("act_warn_fp_missing").format(key=fingerprint_key))
            print(get_string("act_skip_val"))
    
    print(get_string("act_add_footer_vb"))
    
    required_keys = ['partition_size', 'name', 'rollback', 'salt', 'pubkey_sha1', 'algorithm']
    for key in required_keys:
        if key not in vendor_boot_info:
            if key == 'partition_size' and 'data_size' in vendor_boot_info:
                 vendor_boot_info['partition_size'] = vendor_boot_info['data_size']
            else:
                raise KeyError(get_string("act_err_avb_key_missing").format(key=key, name=vendor_boot_bak.name))

    vendor_boot_pubkey = vendor_boot_info.get('pubkey_sha1')
    vendor_boot_key_file = const.KEY_MAP.get(vendor_boot_pubkey)
    if not vendor_boot_key_file:
         raise KeyError(get_string("act_err_unknown_key").format(key=vendor_boot_pubkey))

    _apply_hash_footer(
        image_path=vendor_boot_prc,
        image_info=vendor_boot_info,
        key_file=vendor_boot_key_file,
        new_rollback_index=vendor_boot_info['rollback']
    )
    
    vbmeta_pubkey = vbmeta_info.get('pubkey_sha1')
    key_file = const.KEY_MAP.get(vbmeta_pubkey) 

    print(get_string("act_remake_vbmeta"))
    print(get_string("act_verify_vbmeta_key"))
    if not key_file:
        print(get_string("act_err_vbmeta_key_mismatch").format(key=vbmeta_pubkey))
        raise KeyError(get_string("act_err_unknown_key").format(key=vbmeta_pubkey))
    print(get_string("act_key_matched").format(name=key_file.name))

    print(get_string("act_remaking_vbmeta"))
    vbmeta_img = const.BASE_DIR / "vbmeta.img"
    remake_cmd = [
        str(const.PYTHON_EXE), str(const.AVBTOOL_PY), "make_vbmeta_image",
        "--output", str(vbmeta_img),
        "--key", str(key_file),
        "--algorithm", vbmeta_info['algorithm'],
        "--padding_size", "8192",
        "--flags", vbmeta_info.get('flags', '0'),
        "--rollback_index", vbmeta_info.get('rollback', '0'),
        "--include_descriptors_from_image", str(vbmeta_bak),
        "--include_descriptors_from_image", str(vendor_boot_prc) 
    ]
        
    utils.run_command(remake_cmd)
    print()

    print(get_string("act_finalize"))
    print(get_string("act_rename_final"))
    final_vendor_boot = const.BASE_DIR / "vendor_boot.img"
    shutil.move(const.BASE_DIR / "vendor_boot_prc.img", final_vendor_boot)

    final_images = [final_vendor_boot, const.BASE_DIR / "vbmeta.img"]

    print(get_string("act_move_final").format(dir=const.OUTPUT_DIR.name))
    const.OUTPUT_DIR.mkdir(exist_ok=True)
    for img in final_images:
        if img.exists(): 
            shutil.move(img, const.OUTPUT_DIR / img.name)

    print(get_string("act_move_backup").format(dir=const.BACKUP_DIR.name))
    const.BACKUP_DIR.mkdir(exist_ok=True)
    for bak_file in const.BASE_DIR.glob("*.bak.img"):
        shutil.move(bak_file, const.BACKUP_DIR / bak_file.name)
    print()

    print("=" * 61)
    print(get_string("act_success"))
    print(get_string("act_final_saved").format(dir=const.OUTPUT_DIR.name))
    print("=" * 61)

def select_country_code(prompt_message: str = "Please select a country from the list below:") -> str:
    print(get_string("act_prompt_msg").format(msg=prompt_message.upper()))

    if not const.SORTED_COUNTRY_CODES:
        print(get_string("act_err_codes_missing"), file=sys.stderr)
        raise ImportError(get_string("act_err_codes_missing_exc"))

    sorted_countries = const.SORTED_COUNTRY_CODES
    
    num_cols = 3
    col_width = 38 
    
    line_width = col_width * num_cols
    print("-" * line_width)
    
    for i in range(0, len(sorted_countries), num_cols):
        line = []
        for j in range(num_cols):
            idx = i + j
            if idx < len(sorted_countries):
                code, name = sorted_countries[idx]
                line.append(f"{idx+1:3d}. {name} ({code})".ljust(col_width))
        print("".join(line))
    print("-" * line_width)

    while True:
        try:
            choice = input(get_string("act_enter_num").format(len=len(sorted_countries)))
            choice_idx = int(choice) - 1
            if 0 <= choice_idx < len(sorted_countries):
                selected_code = sorted_countries[choice_idx][0]
                selected_name = sorted_countries[choice_idx][1]
                print(get_string("act_selected").format(name=selected_name, code=selected_code))
                return selected_code
            else:
                print(get_string("act_invalid_num"))
        except ValueError:
            print(get_string("act_invalid_input"))
        except (KeyboardInterrupt, EOFError):
            print(get_string("act_select_cancel"))
            raise KeyboardInterrupt(get_string("act_select_cancel"))

def edit_devinfo_persist() -> None:
    print(get_string("act_start_dp_patch"))
    
    print(get_string("act_wait_dp"))
    const.BACKUP_DIR.mkdir(exist_ok=True) 

    devinfo_img_src = const.BACKUP_DIR / "devinfo.img"
    persist_img_src = const.BACKUP_DIR / "persist.img"
    
    devinfo_img = const.BASE_DIR / "devinfo.img"
    persist_img = const.BASE_DIR / "persist.img"

    if not devinfo_img_src.exists() and not persist_img_src.exists():
        prompt = get_string("act_prompt_dp").format(dir=const.BACKUP_DIR.name)
        while not devinfo_img_src.exists() and not persist_img_src.exists():
            if platform.system() == "Windows":
                os.system('cls')
            else:
                os.system('clear')
            print(get_string("act_wait_files_title"))
            print(prompt)
            print(get_string("act_place_one_file").format(dir=const.BACKUP_DIR.name))
            print(" - devinfo.img")
            print(" - persist.img")
            print(get_string("act_press_enter"))
            try:
                input()
            except EOFError:
                raise RuntimeError(get_string('process_cancelled'))

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
    print(get_string("act_files_backed_up").format(dir=backup_critical_dir.name))

    print(get_string("act_clean_dp_out").format(dir=const.OUTPUT_DP_DIR.name))
    if const.OUTPUT_DP_DIR.exists():
        shutil.rmtree(const.OUTPUT_DP_DIR)
    const.OUTPUT_DP_DIR.mkdir(exist_ok=True)

    print(get_string("act_detect_codes"))
    detected_codes = detect_region_codes()
    
    status_messages = []
    files_found = 0
    
    display_order = ["persist.img", "devinfo.img"]
    
    for fname in display_order:
        if fname in detected_codes:
            code = detected_codes[fname]
            display_name = Path(fname).stem 
            
            if code:
                status_messages.append(f"{display_name}: {code}XX")
                files_found += 1
            else:
                status_messages.append(f"{display_name}: null")
    
    print(get_string("act_detect_result").format(res=', '.join(status_messages)))
    
    if files_found == 0:
        print(get_string("act_no_codes_skip"))
        devinfo_img.unlink(missing_ok=True)
        persist_img.unlink(missing_ok=True)
        return

    print(get_string("act_ask_change_code"))
    choice = ""
    while choice not in ['y', 'n']:
        choice = input(get_string("act_enter_yn")).lower().strip()

    if choice == 'n':
        print(get_string("act_op_cancel"))
        
        devinfo_img.unlink(missing_ok=True)
        persist_img.unlink(missing_ok=True)
        
        print(get_string("act_safety_remove"))
        (const.IMAGE_DIR / "devinfo.img").unlink(missing_ok=True)
        (const.IMAGE_DIR / "persist.img").unlink(missing_ok=True)
        return

    if choice == 'y':
        target_map = detected_codes.copy()
        replacement_code = select_country_code(get_string("act_select_new_code"))
        patch_region_codes(replacement_code, target_map)

        modified_devinfo = const.BASE_DIR / "devinfo_modified.img"
        modified_persist = const.BASE_DIR / "persist_modified.img"
        
        if modified_devinfo.exists():
            shutil.move(modified_devinfo, const.OUTPUT_DP_DIR / "devinfo.img")
        if modified_persist.exists():
            shutil.move(modified_persist, const.OUTPUT_DP_DIR / "persist.img")
            
        print(get_string("act_dp_moved").format(dir=const.OUTPUT_DP_DIR.name))
        
        devinfo_img.unlink(missing_ok=True)
        persist_img.unlink(missing_ok=True)
        
        print("\n" + "=" * 61)
        print(get_string("act_success"))
        print(get_string("act_dp_ready").format(dir=const.OUTPUT_DP_DIR.name))
        print("=" * 61)