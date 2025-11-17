import os
import platform
import shutil
import subprocess
import sys
import time
import re
from pathlib import Path
from typing import Optional, Dict

from .. import constants as const
from .. import utils, device, downloader
from ..downloader import ensure_magiskboot, get_kernel_version_from_adb
from ..errors import ToolError
from ..partition import ensure_params_or_fail
from .system import detect_active_slot_robust
from ..patch.root import patch_boot_with_root_algo
from ..patch.avb import process_boot_image_avb, extract_image_avb_info
from ..i18n import get_string

def _patch_lkm_via_app(
    dev: device.DeviceController,
    work_dir: Path,
    img_name: str
) -> Optional[Path]:
    print(get_string("act_check_ksu"))
    downloader.download_ksu_apk(const.BASE_DIR)

    ksu_apks = list(const.BASE_DIR.glob("*spoofed*.apk"))
    if not ksu_apks:
        print(get_string("act_skip_ksu"))
        return None
    
    apk_path = ksu_apks[0]
    print(get_string("act_install_ksu").format(name=apk_path.name))
    try:
        utils.run_command([str(const.ADB_EXE), "install", "-r", str(apk_path)])
        print(get_string("act_ksu_ok"))
    except Exception as e:
        print(get_string("act_err_ksu").format(e=e))
        print(get_string("act_root_anyway"))
    
    print(get_string("act_push_init_boot"))
    local_img_path = work_dir / img_name
    remote_img_path = f"/sdcard/{img_name}"
    try:
        utils.run_command([str(const.ADB_EXE), "push", str(local_img_path), remote_img_path])
    except Exception as e:
        print(get_string("act_err_push_init_boot").format(e=e))
        return None
    
    print(get_string("act_prompt_patch_app"))
    print(get_string("utils_press_enter"))
    try:
        input()
    except EOFError:
        raise RuntimeError(get_string('process_cancelled'))
    
    print(get_string("act_find_patched_file"))
    try:
        list_cmd = [str(const.ADB_EXE), "shell", "ls", "-t", "/sdcard/Download/kernelsu_next_patched_*.img"]
        result = utils.run_command(list_cmd, capture=True, check=False)
        
        if result.returncode != 0 or not result.stdout.strip():
            print(get_string("act_err_no_patched_files"))
            return None

        files = result.stdout.strip().splitlines()
        latest_file_remote = files[0].strip()
        
        if not latest_file_remote:
             print(get_string("act_err_no_patched_files"))
             return None

        print(get_string("act_pull_patched_file").format(file=latest_file_remote))
        
        final_path = const.BASE_DIR / "init_boot.root.img"
        if final_path.exists():
            final_path.unlink()
        
        utils.run_command([str(const.ADB_EXE), "pull", latest_file_remote, str(final_path)])
        
        if not final_path.exists():
            print(get_string("act_err_pull_failed"))
            return None
            
        return final_path

    except Exception as e:
        print(get_string("act_err_pull_process").format(e=e))
        return None

def root_boot_only(gki: bool = False) -> None:
    img_name = "boot.img" if gki else "init_boot.img"
    bak_name = "boot.bak.img" if gki else "init_boot.bak.img"
    out_dir = const.OUTPUT_ROOT_DIR if gki else const.OUTPUT_ROOT_LKM_DIR
    out_dir_name = const.OUTPUT_ROOT_DIR.name if gki else const.OUTPUT_ROOT_LKM_DIR.name
    
    vbmeta_img_name = "vbmeta.img"
    vbmeta_bak_name = "vbmeta.bak.img"

    if gki:
        wait_prompt_key = "act_prompt_boot"
        success_msg_key = "act_root_saved"
        fail_msg_key = "act_err_root_fail"
        err_missing_key = "act_err_boot_missing"
    else:
        wait_prompt_key = "act_prompt_init_boot_app"
        success_msg_key = "act_root_saved_lkm"
        fail_msg_key = "act_err_root_fail_lkm"
        err_missing_key = "act_err_init_boot_missing"

    wait_prompt = get_string(wait_prompt_key)
    success_msg = get_string(success_msg_key)
    fail_msg = get_string(fail_msg_key)
    err_missing = get_string(err_missing_key)

    print(get_string("act_clean_root_out").format(dir=out_dir_name))
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(exist_ok=True)
    print()
    
    utils.check_dependencies()

    print(get_string("act_wait_boot") if gki else get_string("act_wait_init_boot"))
    const.IMAGE_DIR.mkdir(exist_ok=True) 
    required_files = [img_name]
    
    if not gki:
        required_files.append(vbmeta_img_name)
        wait_prompt = wait_prompt.replace(f"'{img_name}'", f"'{img_name}' and '{vbmeta_img_name}'")

    prompt = wait_prompt.format(name=const.IMAGE_DIR.name)
    utils.wait_for_files(const.IMAGE_DIR, required_files, prompt)
    
    boot_img_src = const.IMAGE_DIR / img_name
    boot_img = const.BASE_DIR / img_name
    
    try:
        shutil.copy(boot_img_src, boot_img)
        print(get_string("act_copy_boot").format(name=boot_img_src.name))
    except (IOError, OSError) as e:
        print(get_string("act_err_copy_boot").format(name=boot_img_src.name, e=e), file=sys.stderr)
        raise ToolError(get_string("act_err_copy_boot").format(name=boot_img_src.name, e=e))

    if not gki:
        vbmeta_img_src = const.IMAGE_DIR / vbmeta_img_name
        vbmeta_img = const.BASE_DIR / vbmeta_img_name
        try:
            shutil.copy(vbmeta_img_src, vbmeta_img)
            print(get_string("act_copy_boot").format(name=vbmeta_img_src.name))
        except (IOError, OSError) as e:
            print(get_string("act_err_copy_boot").format(name=vbmeta_img_src.name, e=e), file=sys.stderr)
            raise ToolError(get_string("act_err_copy_boot").format(name=vbmeta_img_src.name, e=e))

    if not boot_img.exists():
        print(err_missing)
        raise ToolError(err_missing)

    shutil.copy(boot_img, const.BASE_DIR / bak_name)
    print(get_string("act_backup_boot"))
    if not gki:
        shutil.copy(vbmeta_img, const.BASE_DIR / vbmeta_bak_name)
        print(get_string("act_backup_boot"))

    patched_boot_path = None
    patched_vbmeta_path = None
    lkm_kernel_version: Optional[str] = None

    with utils.temporary_workspace(const.WORK_DIR):
        shutil.copy(boot_img, const.WORK_DIR / img_name)
        boot_img.unlink()
        if not gki:
            vbmeta_img.unlink()
        
        if gki:
            magiskboot_exe = utils.get_platform_executable("magiskboot")
            ensure_magiskboot()
            if platform.system() != "Windows":
                os.chmod(magiskboot_exe, 0o755)
            patched_boot_path = patch_boot_with_root_algo(const.WORK_DIR, magiskboot_exe, dev=None, gki=True)
        else:
            try:
                dev = device.DeviceController(skip_adb=False)
                dev.wait_for_adb()
                lkm_kernel_version = get_kernel_version_from_adb(dev)
                patched_boot_path = _patch_lkm_via_app(dev, const.WORK_DIR, img_name)
            except Exception as e:
                print(get_string("act_err_adb_process").format(e=e), file=sys.stderr)
                patched_boot_path = None

    if patched_boot_path and patched_boot_path.exists():
        print(get_string("act_finalize_root"))
        final_boot_img = out_dir / img_name
        
        process_boot_image_avb(patched_boot_path, gki=gki)

        if not gki:
            print(get_string("act_remake_vbmeta"))
            vbmeta_bak = const.BASE_DIR / vbmeta_bak_name
            vbmeta_info = extract_image_avb_info(vbmeta_bak)
            
            vbmeta_pubkey = vbmeta_info.get('pubkey_sha1')
            key_file = const.KEY_MAP.get(vbmeta_pubkey) 

            print(get_string("act_verify_vbmeta_key"))
            if not key_file:
                print(get_string("act_err_vbmeta_key_mismatch").format(key=vbmeta_pubkey))
                raise KeyError(get_string("act_err_unknown_key").format(key=vbmeta_pubkey))
            print(get_string("act_key_matched").format(name=key_file.name))
            
            print(get_string("act_remaking_vbmeta"))
            patched_vbmeta_path = const.BASE_DIR / "vbmeta.root.img"
            remake_cmd = [
                str(const.PYTHON_EXE), str(const.AVBTOOL_PY), "make_vbmeta_image",
                "--output", str(patched_vbmeta_path),
                "--key", str(key_file),
                "--algorithm", vbmeta_info['algorithm'],
                "--padding_size", "8192",
                "--flags", vbmeta_info.get('flags', '0'),
                "--rollback_index", vbmeta_info.get('rollback', '0'),
                "--include_descriptors_from_image", str(vbmeta_bak),
                "--include_descriptors_from_image", str(patched_boot_path) 
            ]
            utils.run_command(remake_cmd)
            print()

        print(get_string("act_move_root_final").format(dir=out_dir_name))
        shutil.move(patched_boot_path, final_boot_img)
        if patched_vbmeta_path and patched_vbmeta_path.exists():
            shutil.move(patched_vbmeta_path, out_dir / vbmeta_img_name)

        print(get_string("act_move_root_backup").format(dir=const.BACKUP_DIR.name))
        const.BACKUP_DIR.mkdir(exist_ok=True)
        for bak_file in const.BASE_DIR.glob("*.bak.img"):
            shutil.move(bak_file, const.BACKUP_DIR / bak_file.name)
        print()

        print("=" * 61)
        print(get_string("act_success"))
        print(success_msg.format(dir=out_dir_name))
        if not gki:
            print(f"  Patched {vbmeta_img_name} saved to '{out_dir_name}'.")
        print("=" * 61)
    else:
        print(fail_msg, file=sys.stderr)

def root_device(dev: device.DeviceController, gki: bool = False) -> None:
    print(get_string("act_start_root"))
    
    img_name = "boot.img" if gki else "init_boot.img"
    bak_name = "boot.bak.img" if gki else "init_boot.bak.img"
    out_dir = const.OUTPUT_ROOT_DIR if gki else const.OUTPUT_ROOT_LKM_DIR
    bak_dir = const.BACKUP_BOOT_DIR if gki else const.BACKUP_INIT_BOOT_DIR
    
    vbmeta_img_name = "vbmeta.img"
    vbmeta_bak_name = "vbmeta.bak.img"
    
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(exist_ok=True)
    bak_dir.mkdir(exist_ok=True)

    utils.check_dependencies()
    
    magiskboot_exe = utils.get_platform_executable("magiskboot")
    ensure_magiskboot()

    print(get_string("act_root_step1"))
    if not dev.skip_adb:
        dev.wait_for_adb()

    active_slot = detect_active_slot_robust(dev)
    
    lkm_kernel_version: Optional[str] = None
    if not gki:
        if not dev.skip_adb:
            try:
                lkm_kernel_version = get_kernel_version_from_adb(dev)
            except Exception as e:
                print(f"[!] Warning: Could not get kernel version via ADB: {e}", file=sys.stderr)
                print("[!] Will attempt to get it later, but this may fail if device is rebooting.")
        else:
            print("[!] LKM root with 'Skip ADB' is not supported as it needs the kernel version.", file=sys.stderr)
            raise ToolError("LKM root requires ADB to get kernel version.")

    target_partition = ""
    target_vbmeta_partition = ""
    if active_slot:
        print(get_string("act_slot_confirmed").format(slot=active_slot))
        target_partition = f"boot{active_slot}" if gki else f"init_boot{active_slot}"
        target_vbmeta_partition = f"vbmeta{active_slot}"
    else:
        print(get_string("act_warn_root_slot"))
        target_partition = "boot" if gki else "init_boot"
        target_vbmeta_partition = "vbmeta"

    if not dev.skip_adb and gki:
        print(get_string("act_check_ksu"))
        downloader.download_ksu_apk(const.BASE_DIR)
        
        ksu_apks = list(const.BASE_DIR.glob("*spoofed*.apk"))
        if ksu_apks:
            apk_path = ksu_apks[0]
            print(get_string("act_install_ksu").format(name=apk_path.name))
            try:
                utils.run_command([str(const.ADB_EXE), "install", "-r", str(apk_path)])
                print(get_string("act_ksu_ok"))
            except Exception as e:
                print(get_string("act_err_ksu").format(e=e))
                print(get_string("act_root_anyway"))
        else:
            print(get_string("act_skip_ksu"))
    
    print(get_string("act_root_step2"))
    port = dev.setup_edl_connection()
    
    try:
        dev.load_firehose_programmer_with_stability(const.EDL_LOADER_FILE, port)
    except Exception as e:
        print(get_string("act_warn_prog_load").format(e=e))

    if gki:
        print(get_string("act_root_step3").format(part=target_partition))
    else:
        print(get_string("act_root_step3_init_boot").format(part=target_partition))
    
    params = None
    params_vbmeta = None
    final_boot_img = out_dir / img_name
    final_vbmeta_img = out_dir / vbmeta_img_name
    
    with utils.temporary_workspace(const.WORKING_BOOT_DIR):
        dumped_boot_img = const.WORKING_BOOT_DIR / img_name
        backup_boot_img = bak_dir / img_name
        base_boot_bak = const.BASE_DIR / bak_name

        dumped_vbmeta_img = const.WORKING_BOOT_DIR / vbmeta_img_name
        backup_vbmeta_img = bak_dir / vbmeta_img_name
        base_vbmeta_bak = const.BASE_DIR / vbmeta_bak_name

        try:
            params = ensure_params_or_fail(target_partition)
            print(get_string("act_found_dump_info").format(xml=params['source_xml'], lun=params['lun'], start=params['start_sector']))
            dev.fh_loader_read_part(
                port=port,
                output_filename=str(dumped_boot_img),
                lun=params['lun'],
                start_sector=params['start_sector'],
                num_sectors=params['num_sectors']
            )
            
            if not gki:
                params_vbmeta = ensure_params_or_fail(target_vbmeta_partition)
                print(get_string("act_found_dump_info").format(xml=params_vbmeta['source_xml'], lun=params_vbmeta['lun'], start=params_vbmeta['start_sector']))
                dev.fh_loader_read_part(
                    port=port,
                    output_filename=str(dumped_vbmeta_img),
                    lun=params_vbmeta['lun'],
                    start_sector=params_vbmeta['start_sector'],
                    num_sectors=params_vbmeta['num_sectors']
                )

            if params.get('size_in_kb'):
                try:
                    expected_size_bytes = int(float(params['size_in_kb']) * 1024)
                    actual_size_bytes = dumped_boot_img.stat().st_size
                    
                    if expected_size_bytes != actual_size_bytes:
                        raise RuntimeError(
                            f"Dumped file size mismatch for '{target_partition}'. "
                            f"Expected: {expected_size_bytes}B, Got: {actual_size_bytes}B"
                        )
                except (ValueError, OSError) as e:
                    print(get_string("act_err_dump").format(part=target_partition, e=e), file=sys.stderr)
                    raise
            
            if gki:
                print(get_string("act_read_boot_ok").format(part=target_partition, file=dumped_boot_img))
            else:
                print(get_string("act_read_init_boot_ok").format(part=target_partition, file=dumped_boot_img))
                print(get_string("act_read_boot_ok").format(part=target_vbmeta_partition, file=dumped_vbmeta_img))
        except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
            print(get_string("act_err_dump").format(part=target_partition, e=e), file=sys.stderr)
            raise

        print(get_string("act_backup_boot_root").format(dir=backup_boot_img.parent.name))
        shutil.copy(dumped_boot_img, backup_boot_img)
        print(get_string("act_temp_backup_avb"))
        shutil.copy(dumped_boot_img, base_boot_bak)
        if not gki:
            shutil.copy(dumped_vbmeta_img, backup_vbmeta_img)
            shutil.copy(dumped_vbmeta_img, base_vbmeta_bak)
        print(get_string("act_backups_done"))

        print(get_string("act_dump_reset"))
        dev.fh_loader_reset(port)
        
        if gki:
            print(get_string("act_root_step4"))
        else:
            print(get_string("act_root_step4_init_boot"))
            
        patched_boot_path = patch_boot_with_root_algo(
            const.WORKING_BOOT_DIR, magiskboot_exe, 
            dev=dev, gki=gki,
            lkm_kernel_version=lkm_kernel_version
        )

        if not (patched_boot_path and patched_boot_path.exists()):
            print(get_string("act_err_root_fail"), file=sys.stderr)
            base_boot_bak.unlink(missing_ok=True)
            if not gki: base_vbmeta_bak.unlink(missing_ok=True)
            raise ToolError(get_string("act_err_root_fail"))

        print(get_string("act_root_step5"))
        try:
            process_boot_image_avb(patched_boot_path, gki=gki)
        except Exception as e:
            print(get_string("act_err_avb_footer").format(e=e), file=sys.stderr)
            base_boot_bak.unlink(missing_ok=True)
            if not gki: base_vbmeta_bak.unlink(missing_ok=True)
            raise

        if not gki:
            print(get_string("act_remake_vbmeta"))
            vbmeta_bak = base_vbmeta_bak
            vbmeta_info = extract_image_avb_info(vbmeta_bak)
            
            vbmeta_pubkey = vbmeta_info.get('pubkey_sha1')
            key_file = const.KEY_MAP.get(vbmeta_pubkey) 

            print(get_string("act_verify_vbmeta_key"))
            if not key_file:
                print(get_string("act_err_vbmeta_key_mismatch").format(key=vbmeta_pubkey))
                raise KeyError(get_string("act_err_unknown_key").format(key=vbmeta_pubkey))
            print(get_string("act_key_matched").format(name=key_file.name))
            
            print(get_string("act_remaking_vbmeta"))
            patched_vbmeta_path = const.BASE_DIR / "vbmeta.root.img"
            remake_cmd = [
                str(const.PYTHON_EXE), str(const.AVBTOOL_PY), "make_vbmeta_image",
                "--output", str(patched_vbmeta_path),
                "--key", str(key_file),
                "--algorithm", vbmeta_info['algorithm'],
                "--padding_size", "8192",
                "--flags", vbmeta_info.get('flags', '0'),
                "--rollback_index", vbmeta_info.get('rollback', '0'),
                "--include_descriptors_from_image", str(vbmeta_bak),
                "--include_descriptors_from_image", str(patched_boot_path) 
            ]
            utils.run_command(remake_cmd)
            print()
            shutil.move(patched_vbmeta_path, final_vbmeta_img)
            print(get_string("act_patched_boot_saved").format(dir=final_vbmeta_img.parent.name))

        shutil.move(patched_boot_path, final_boot_img)
        print(get_string("act_patched_boot_saved").format(dir=final_boot_img.parent.name))

        base_boot_bak.unlink(missing_ok=True)
        if not gki: base_vbmeta_bak.unlink(missing_ok=True)

    if gki:
        print(get_string("act_root_step6").format(part=target_partition))
    else:
        print(get_string("act_root_step6_init_boot").format(part=target_partition))
    
    if not dev.skip_adb:
        print(get_string("act_wait_sys_adb"))
        dev.wait_for_adb()
        print(get_string("act_reboot_edl_flash"))
        port = dev.setup_edl_connection()
    else:
        print(get_string("act_skip_adb_on"))
        print(get_string("act_manual_edl_now"))
        port = dev.wait_for_edl()

    try:
        dev.load_firehose_programmer_with_stability(const.EDL_LOADER_FILE, port)
    except Exception as e:
        print(get_string("act_warn_prog_load").format(e=e))

    if not params:
         params = ensure_params_or_fail(target_partition)
    if not gki:
        if not params_vbmeta:
             params_vbmeta = ensure_params_or_fail(target_vbmeta_partition)

    try:
        dev.fh_loader_write_part(
            port=port,
            image_path=final_boot_img,
            lun=params['lun'],
            start_sector=params['start_sector']
        )
        if gki:
            print(get_string("act_flash_boot_ok").format(part=target_partition))
        else:
            print(get_string("act_flash_init_boot_ok").format(part=target_partition))
            
            dev.fh_loader_write_part(
                port=port,
                image_path=final_vbmeta_img,
                lun=params_vbmeta['lun'],
                start_sector=params_vbmeta['start_sector']
            )
            print(get_string("act_flash_boot_ok").format(part=target_vbmeta_partition))

        print(get_string("act_reset_sys"))
        dev.fh_loader_reset(port)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(get_string("act_err_edl_write").format(e=e), file=sys.stderr)
        raise

    print(get_string("act_root_finish"))

def unroot_device(dev: device.DeviceController) -> None:
    print(get_string("act_start_unroot"))
    
    gki_bak_dir = const.BACKUP_BOOT_DIR
    lkm_bak_dir = const.BACKUP_INIT_BOOT_DIR
    
    gki_boot_file = gki_bak_dir / "boot.img"
    lkm_init_boot_file = lkm_bak_dir / "init_boot.img"
    lkm_vbmeta_file = lkm_bak_dir / "vbmeta.img"
    
    gki_bak_dir.mkdir(exist_ok=True)
    lkm_bak_dir.mkdir(exist_ok=True)
    
    gki_exists = gki_boot_file.exists()
    lkm_exists = lkm_init_boot_file.exists() and lkm_vbmeta_file.exists()
    
    unroot_mode: Optional[str] = None
    
    if gki_exists and lkm_exists:
        os.system('cls' if os.name == 'nt' else 'clear')
        print("\n  " + "=" * 58)
        print(get_string("act_unroot_menu_title"))
        print("  " + "=" * 58 + "\n")
        print(get_string("act_unroot_menu_1_lkm"))
        print(get_string("act_unroot_menu_2_gki"))
        print("\n" + get_string("act_unroot_menu_m"))
        print("\n  " + "=" * 58 + "\n")
        
        while unroot_mode is None:
            choice = input(get_string("act_unroot_menu_prompt")).strip().lower()
            if choice == "1":
                unroot_mode = "lkm"
            elif choice == "2":
                unroot_mode = "gki"
            elif choice == "m":
                print(get_string("act_op_cancel"))
                return
            else:
                print(get_string("act_unroot_menu_invalid"))
                
    elif lkm_exists:
        print(get_string("act_unroot_lkm_detected"))
        unroot_mode = "lkm"
    elif gki_exists:
        print(get_string("act_unroot_gki_detected"))
        unroot_mode = "gki"
    else:
        prompt = get_string("act_unroot_prompt_all").format(
            lkm_dir=lkm_bak_dir.name, 
            gki_dir=gki_bak_dir.name
        )
        
        def check_for_unroot_files(p: Path, f: Optional[list]) -> bool:
            return gki_boot_file.exists() or (lkm_init_boot_file.exists() and lkm_vbmeta_file.exists())
        
        utils._wait_for_resource(const.BASE_DIR, check_for_unroot_files, prompt, None)
        
        if lkm_init_boot_file.exists() and lkm_vbmeta_file.exists():
            unroot_mode = "lkm"
            print(get_string("act_unroot_lkm_detected"))
        else:
            unroot_mode = "gki"
            print(get_string("act_unroot_gki_detected"))

    print(get_string("act_unroot_step1"))
    if not list(const.IMAGE_DIR.glob("rawprogram*.xml")) and not list(const.IMAGE_DIR.glob("*.x")):
         print(get_string("act_err_no_xmls").format(dir=const.IMAGE_DIR.name))
         print(get_string("act_unroot_req_xmls"))
         prompt = get_string("act_prompt_image")
         utils.wait_for_directory(const.IMAGE_DIR, prompt)

    print(get_string("act_unroot_step3"))
    if not dev.skip_adb:
        dev.wait_for_adb()
    
    active_slot = detect_active_slot_robust(dev)
    suffix = active_slot if active_slot else ""

    port = dev.setup_edl_connection()

    try:
        dev.load_firehose_programmer_with_stability(const.EDL_LOADER_FILE, port)
    except Exception as e:
        print(get_string("act_warn_prog_load").format(e=e))

    try:
        if unroot_mode == "lkm":
            target_init_boot = f"init_boot{suffix}"
            target_vbmeta = f"vbmeta{suffix}"
            print(get_string("act_unroot_step4_lkm"))

            params_init = ensure_params_or_fail(target_init_boot)
            print(get_string("act_found_dump_info").format(xml=params_init['source_xml'], lun=params_init['lun'], start=params_init['start_sector']))
            dev.fh_loader_write_part(
                port=port,
                image_path=lkm_init_boot_file,
                lun=params_init['lun'],
                start_sector=params_init['start_sector']
            )
            print(get_string("act_flash_stock_init_boot_ok").format(part=target_init_boot))

            params_vbmeta = ensure_params_or_fail(target_vbmeta)
            print(get_string("act_found_dump_info").format(xml=params_vbmeta['source_xml'], lun=params_vbmeta['lun'], start=params_vbmeta['start_sector']))
            dev.fh_loader_write_part(
                port=port,
                image_path=lkm_vbmeta_file,
                lun=params_vbmeta['lun'],
                start_sector=params_vbmeta['start_sector']
            )
            print(get_string("act_flash_stock_vbmeta_ok").format(part=target_vbmeta))
            
        elif unroot_mode == "gki":
            target_boot = f"boot{suffix}"
            print(get_string("act_unroot_step4_gki").format(part=target_boot))
            
            params = ensure_params_or_fail(target_boot)
            print(get_string("act_found_dump_info").format(xml=params['source_xml'], lun=params['lun'], start=params['start_sector']))
            
            dev.fh_loader_write_part(
                port=port,
                image_path=gki_boot_file,
                lun=params['lun'],
                start_sector=params['start_sector']
            )
            print(get_string("act_flash_stock_boot_ok").format(part=target_boot))
        
        print(get_string("act_reset_sys"))
        dev.fh_loader_reset(port)
        
    except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
        print(get_string("act_err_edl_write").format(e=e), file=sys.stderr)
        raise

    print(get_string("act_unroot_finish"))