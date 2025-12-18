import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Optional

from . import edl
from .. import constants as const
from .. import utils, device, downloader
from ..downloader import ensure_magiskboot
from ..errors import ToolError
from ..partition import ensure_params_or_fail
from .system import detect_active_slot_robust
from ..patch.root import patch_boot_with_root_algo
from ..patch.avb import process_boot_image_avb, extract_image_avb_info, rebuild_vbmeta_with_chained_images, _apply_hash_footer
from ..i18n import get_string

def _patch_lkm_via_app(
    dev: device.DeviceController,
    work_dir: Path,
    img_name: str,
    root_type: str = "ksu"
) -> Optional[Path]:
    is_sukisu = (root_type == "sukisu")
    check_key = "act_check_sukisu" if is_sukisu else "act_check_ksu"
    
    utils.ui.echo(get_string(check_key))
    
    if is_sukisu:
        downloader.download_sukisu_manager(const.BASE_DIR)
        ksu_apks = list(const.BASE_DIR.glob("SukiSU*.apk"))
    else:
        downloader.download_ksu_apk(const.BASE_DIR)
        ksu_apks = list(const.BASE_DIR.glob("*spoofed*.apk"))
        if not ksu_apks:
            candidates = list(const.BASE_DIR.glob("*.apk"))
            ksu_apks = [f for f in candidates if "SukiSU" not in f.name]

    if not ksu_apks:
        skip_key = "act_skip_sukisu" if is_sukisu else "act_skip_ksu"
        utils.ui.echo(get_string(skip_key))
        return None
    
    apk_path = ksu_apks[0]
    utils.ui.echo(get_string("act_install_ksu").format(name=apk_path.name))
    try:
        dev.install_apk(str(apk_path))
        utils.ui.echo(get_string("act_ksu_ok"))
    except Exception as e:
        utils.ui.echo(get_string("act_err_ksu").format(e=e))
        utils.ui.echo(get_string("act_root_anyway"))
    
    utils.ui.echo(get_string("act_push_init_boot"))
    local_img_path = work_dir / img_name
    remote_img_path = f"/sdcard/{img_name}"
    try:
        dev.push_file(str(local_img_path), remote_img_path)
    except Exception as e:
        utils.ui.echo(get_string("act_err_push_init_boot").format(e=e))
        return None
    
    prompt_key = "act_prompt_patch_app_sukisu" if is_sukisu else "act_prompt_patch_app"
    utils.ui.echo(get_string(prompt_key))
    utils.ui.echo(get_string("press_enter_to_continue"))
    try:
        utils.ui.prompt()
    except EOFError:
        raise RuntimeError(get_string('act_op_cancel'))
    
    utils.ui.echo(get_string("act_find_patched_file"))
    try:
        if is_sukisu:
             # Sukisu saves as kernelsu_patched_*.img
             cmd_output = dev.adb_shell("ls -t /sdcard/Download/kernelsu_patched_*.img")
        else:
             cmd_output = dev.adb_shell("ls -t /sdcard/Download/kernelsu_next_patched_*.img")
        
        if not cmd_output.strip():
            utils.ui.echo(get_string("act_err_no_patched_files"))
            return None

        files = cmd_output.strip().splitlines()
        latest_file_remote = files[0].strip()
        
        if not latest_file_remote:
             utils.ui.echo(get_string("act_err_no_patched_files"))
             return None

        utils.ui.echo(get_string("act_pull_patched_file").format(file=latest_file_remote))
        
        final_path = const.BASE_DIR / "init_boot.root.img"
        if final_path.exists():
            final_path.unlink()

        dev.pull_file(latest_file_remote, str(final_path))
        
        if not final_path.exists():
            utils.ui.echo(get_string("act_err_pull_failed"))
            return None
            
        return final_path

    except Exception as e:
        utils.ui.echo(get_string("act_err_pull_process").format(e=e))
        return None

def patch_root_image_file(gki: bool = False, root_type: str = "ksu") -> None:
    img_name = const.FN_BOOT if gki else const.FN_INIT_BOOT
    bak_name = const.FN_BOOT_BAK if gki else const.FN_INIT_BOOT_BAK
    out_dir = const.OUTPUT_ROOT_DIR if gki else const.OUTPUT_ROOT_LKM_DIR
    out_dir_name = const.OUTPUT_ROOT_DIR.name if gki else const.OUTPUT_ROOT_LKM_DIR.name
    
    vbmeta_img_name = const.FN_VBMETA
    vbmeta_bak_name = const.FN_VBMETA_BAK

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

    utils.ui.echo(get_string("act_clean_root_out").format(dir=out_dir_name))
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(exist_ok=True)
    utils.ui.echo("")
    
    utils.check_dependencies()

    utils.ui.echo(get_string("act_wait_boot") if gki else get_string("act_wait_init_boot"))
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
        utils.ui.echo(get_string("act_copy_boot").format(name=boot_img_src.name))
    except (IOError, OSError) as e:
        utils.ui.error(get_string("act_err_copy_boot").format(name=boot_img_src.name, e=e))
        raise ToolError(get_string("act_err_copy_boot").format(name=boot_img_src.name, e=e))

    if not gki:
        vbmeta_img_src = const.IMAGE_DIR / vbmeta_img_name
        vbmeta_img = const.BASE_DIR / vbmeta_img_name
        try:
            shutil.copy(vbmeta_img_src, vbmeta_img)
            utils.ui.echo(get_string("act_copy_boot").format(name=vbmeta_img_src.name))
        except (IOError, OSError) as e:
            utils.ui.error(get_string("act_err_copy_boot").format(name=vbmeta_img_src.name, e=e))
            raise ToolError(get_string("act_err_copy_boot").format(name=vbmeta_img_src.name, e=e))

    if not boot_img.exists():
        utils.ui.echo(err_missing)
        raise ToolError(err_missing)

    utils.ui.echo(get_string("act_backup_boot"))
    shutil.copy(boot_img, const.BASE_DIR / bak_name)
    if not gki:
        shutil.copy(vbmeta_img, const.BASE_DIR / vbmeta_bak_name)

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
            patched_boot_path = patch_boot_with_root_algo(const.WORK_DIR, magiskboot_exe, dev=None, gki=True)
        else:
            try:
                dev = device.DeviceController(skip_adb=False)
                dev.wait_for_adb()
                lkm_kernel_version = dev.get_kernel_version()
                patched_boot_path = _patch_lkm_via_app(dev, const.WORK_DIR, img_name, root_type=root_type)
            except Exception as e:
                utils.ui.error(get_string("act_err_adb_process").format(e=e))
                patched_boot_path = None

    if patched_boot_path and patched_boot_path.exists():
        utils.ui.echo(get_string("act_finalize_root"))
        final_boot_img = out_dir / img_name
        
        process_boot_image_avb(patched_boot_path, gki=gki)

        if not gki:
            vbmeta_bak = const.BASE_DIR / vbmeta_bak_name
            patched_vbmeta_path = const.BASE_DIR / const.FN_VBMETA_ROOT
            
            rebuild_vbmeta_with_chained_images(
                output_path=patched_vbmeta_path,
                original_vbmeta_path=vbmeta_bak,
                chained_images=[patched_boot_path]
            )
            utils.ui.echo("")

        utils.ui.echo(get_string("act_move_root_final").format(dir=out_dir_name))
        shutil.move(patched_boot_path, final_boot_img)
        if patched_vbmeta_path and patched_vbmeta_path.exists():
            shutil.move(patched_vbmeta_path, out_dir / vbmeta_img_name)

        utils.ui.echo(get_string("act_move_root_backup").format(dir=const.BACKUP_DIR.name))
        const.BACKUP_DIR.mkdir(exist_ok=True)
        for bak_file in const.BASE_DIR.glob("*.bak.img"):
            shutil.move(bak_file, const.BACKUP_DIR / bak_file.name)
        utils.ui.echo("")

        utils.ui.echo("  " + "=" * 78)
        utils.ui.echo(get_string("act_success"))
        utils.ui.echo(success_msg.format(dir=out_dir_name))
        if not gki:
            utils.ui.echo(get_string("act_root_saved_vbmeta_lkm").format(name=vbmeta_img_name, dir=out_dir_name))
        
        utils.ui.echo("\n" + get_string("act_root_manual_flash_notice"))
        utils.ui.echo("  " + "=" * 78)
    else:
        utils.ui.error(fail_msg)

def root_device(dev: device.DeviceController, gki: bool = False, root_type: str = "ksu") -> None:
    utils.ui.echo(get_string("act_start_root"))
    
    img_name = const.FN_BOOT if gki else const.FN_INIT_BOOT
    bak_name = const.FN_BOOT_BAK if gki else const.FN_INIT_BOOT_BAK
    out_dir = const.OUTPUT_ROOT_DIR if gki else const.OUTPUT_ROOT_LKM_DIR
    bak_dir = const.BACKUP_BOOT_DIR if gki else const.BACKUP_INIT_BOOT_DIR
    
    vbmeta_img_name = const.FN_VBMETA
    vbmeta_bak_name = const.FN_VBMETA_BAK
    
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(exist_ok=True)
    bak_dir.mkdir(exist_ok=True)

    utils.check_dependencies()
    
    edl.ensure_edl_requirements()

    magiskboot_exe = utils.get_platform_executable("magiskboot")
    ensure_magiskboot()

    utils.ui.echo(get_string("act_root_step1"))
    if not dev.skip_adb:
        dev.wait_for_adb()

    active_slot = detect_active_slot_robust(dev)
    
    lkm_kernel_version: Optional[str] = None

    if not gki:
        if not dev.skip_adb:
            try:
                lkm_kernel_version = dev.get_kernel_version()
            except Exception as e:
                utils.ui.error(get_string("act_root_warn_lkm_kver_fail").format(e=e))
                utils.ui.error(get_string("act_root_warn_lkm_kver_retry"))
        else:
            utils.ui.error(get_string("act_root_err_lkm_skip_adb"))
            raise ToolError(get_string("act_root_err_lkm_skip_adb_exc"))

    target_partition = ""
    target_vbmeta_partition = ""
    if active_slot:
        utils.ui.echo(get_string("act_slot_confirmed").format(slot=active_slot))
        target_partition = f"boot{active_slot}" if gki else f"init_boot{active_slot}"
        target_vbmeta_partition = f"vbmeta{active_slot}"
    else:
        utils.ui.echo(get_string("act_warn_root_slot"))
        target_partition = "boot" if gki else "init_boot"
        target_vbmeta_partition = "vbmeta"

    if not dev.skip_adb:
        is_sukisu = (root_type == "sukisu")
        check_key = "act_check_sukisu" if is_sukisu else "act_check_ksu"
        utils.ui.echo(get_string(check_key))
        
        if is_sukisu:
            downloader.download_sukisu_manager(const.BASE_DIR)
            ksu_apks = list(const.BASE_DIR.glob("SukiSU*.apk"))
        else:
            downloader.download_ksu_apk(const.BASE_DIR)
            ksu_apks = list(const.BASE_DIR.glob("*spoofed*.apk"))
            if not ksu_apks:
                candidates = list(const.BASE_DIR.glob("*.apk"))
                ksu_apks = [f for f in candidates if "SukiSU" not in f.name]
        
        if ksu_apks:
            apk_path = ksu_apks[0]
            utils.ui.echo(get_string("act_install_ksu").format(name=apk_path.name))
            try:
                dev.install_apk(str(apk_path))
                utils.ui.echo(get_string("act_ksu_ok"))
            except Exception as e:
                utils.ui.echo(get_string("act_err_ksu").format(e=e))
                utils.ui.echo(get_string("act_root_anyway"))
        else:
            skip_key = "act_skip_sukisu" if is_sukisu else "act_skip_ksu"
            utils.ui.echo(get_string(skip_key))
    
    utils.ui.echo(get_string("act_root_step2"))
    port = dev.setup_edl_connection()
    
    try:
        dev.load_firehose_programmer_with_stability(const.EDL_LOADER_FILE, port)
    except Exception as e:
        utils.ui.echo(get_string("act_warn_prog_load").format(e=e))

    if gki:
        utils.ui.echo(get_string("act_root_step3").format(part=target_partition))
    else:
        utils.ui.echo(get_string("act_root_step3_init_boot").format(part=target_partition))
    
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
            utils.ui.echo(get_string("act_found_dump_info").format(xml=params['source_xml'], lun=params['lun'], start=params['start_sector']))
            dev.edl_read_partition(
                port=port,
                output_filename=str(dumped_boot_img),
                lun=params['lun'],
                start_sector=params['start_sector'],
                num_sectors=params['num_sectors']
            )
            
            if not gki:
                params_vbmeta = ensure_params_or_fail(target_vbmeta_partition)
                utils.ui.echo(get_string("act_found_dump_info").format(xml=params_vbmeta['source_xml'], lun=params_vbmeta['lun'], start=params_vbmeta['start_sector']))
                dev.edl_read_partition(
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
                            get_string("act_err_dump_mismatch").format(
                                part=target_partition,
                                expected=expected_size_bytes,
                                actual=actual_size_bytes
                            )
                        )
                except (ValueError, OSError) as e:
                    utils.ui.error(get_string("act_err_dump").format(part=target_partition, e=f"Size validation error: {e}"))
                    raise
            
            if gki:
                utils.ui.echo(get_string("act_read_boot_ok").format(part=target_partition, file=dumped_boot_img))
            else:
                utils.ui.echo(get_string("act_read_init_boot_ok").format(part=target_partition, file=dumped_boot_img))
                utils.ui.echo(get_string("act_read_boot_ok").format(part=target_vbmeta_partition, file=dumped_vbmeta_img))
        except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
            utils.ui.error(get_string("act_err_dump").format(part=target_partition, e=e))
            raise

        utils.ui.echo(get_string("act_backup_boot_root").format(dir=backup_boot_img.parent.name))
        shutil.copy(dumped_boot_img, backup_boot_img)
        utils.ui.echo(get_string("act_temp_backup_avb"))
        shutil.copy(dumped_boot_img, base_boot_bak)
        if not gki:
            shutil.copy(dumped_vbmeta_img, backup_vbmeta_img)
            shutil.copy(dumped_vbmeta_img, base_vbmeta_bak)
        utils.ui.echo(get_string("act_backups_done"))

        utils.ui.echo(get_string("act_dump_reset"))
        dev.edl_reset(port)
        
        if gki:
            utils.ui.echo(get_string("act_root_step4"))
        else:
            utils.ui.echo(get_string("act_root_step4_init_boot"))
            
        patched_boot_path = patch_boot_with_root_algo(
            const.WORKING_BOOT_DIR, magiskboot_exe, 
            dev=dev, gki=gki,
            lkm_kernel_version=lkm_kernel_version,
            root_type=root_type
        )

        if not (patched_boot_path and patched_boot_path.exists()):
            utils.ui.error(get_string("act_err_root_fail"))
            base_boot_bak.unlink(missing_ok=True)
            if not gki: base_vbmeta_bak.unlink(missing_ok=True)
            raise ToolError(get_string("act_err_root_fail"))

        utils.ui.echo(get_string("act_root_step5"))
        try:
            process_boot_image_avb(patched_boot_path, gki=gki)
        except Exception as e:
            utils.ui.error(get_string("act_err_avb_footer").format(e=e))
            base_boot_bak.unlink(missing_ok=True)
            if not gki: base_vbmeta_bak.unlink(missing_ok=True)
            raise

        if not gki:
            vbmeta_bak = base_vbmeta_bak
            patched_vbmeta_path = const.BASE_DIR / const.FN_VBMETA_ROOT
            
            rebuild_vbmeta_with_chained_images(
                output_path=patched_vbmeta_path,
                original_vbmeta_path=vbmeta_bak,
                chained_images=[patched_boot_path]
            )
            utils.ui.echo("")
            shutil.move(patched_vbmeta_path, final_vbmeta_img)
            utils.ui.echo(get_string("act_patched_boot_saved").format(dir=final_vbmeta_img.parent.name))

        shutil.move(patched_boot_path, final_boot_img)
        utils.ui.echo(get_string("act_patched_boot_saved").format(dir=final_boot_img.parent.name))

        base_boot_bak.unlink(missing_ok=True)
        if not gki: base_vbmeta_bak.unlink(missing_ok=True)

    if gki:
        utils.ui.echo(get_string("act_root_step6").format(part=target_partition))
    else:
        utils.ui.echo(get_string("act_root_step6_init_boot").format(part=target_partition))
    
    if not dev.skip_adb:
        utils.ui.echo(get_string("act_wait_sys_adb"))
        dev.wait_for_adb()
        utils.ui.echo(get_string("act_reboot_edl_flash"))
        port = dev.setup_edl_connection()
    else:
        utils.ui.echo(get_string("act_skip_adb_on"))
        utils.ui.echo(get_string("act_manual_edl_now"))
        port = dev.wait_for_edl()

    try:
        dev.load_firehose_programmer_with_stability(const.EDL_LOADER_FILE, port)
    except Exception as e:
        utils.ui.echo(get_string("act_warn_prog_load").format(e=e))

    try:
        edl.flash_partition_target(dev, port, target_partition, final_boot_img)

        if not gki:
            edl.flash_partition_target(dev, port, target_vbmeta_partition, final_vbmeta_img)
            utils.ui.echo(get_string("act_flash_boot_ok").format(part=target_vbmeta_partition))

        if gki:
            utils.ui.echo(get_string("act_flash_boot_ok").format(part=target_partition))
        else:
            utils.ui.echo(get_string("act_flash_init_boot_ok").format(part=target_partition))

        utils.ui.echo(get_string("act_reset_sys"))
        dev.edl_reset(port)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        utils.ui.error(get_string("act_err_edl_write").format(e=e))
        raise

    utils.ui.echo(get_string("act_root_finish"))

def unroot_device(dev: device.DeviceController) -> None:
    utils.ui.echo(get_string("act_start_unroot"))
    
    gki_bak_dir = const.BACKUP_BOOT_DIR
    lkm_bak_dir = const.BACKUP_INIT_BOOT_DIR
    
    gki_boot_file = gki_bak_dir / const.FN_BOOT
    lkm_init_boot_file = lkm_bak_dir / const.FN_INIT_BOOT
    lkm_vbmeta_file = lkm_bak_dir / const.FN_VBMETA
    
    gki_bak_dir.mkdir(exist_ok=True)
    lkm_bak_dir.mkdir(exist_ok=True)
    
    gki_exists = gki_boot_file.exists()
    lkm_exists = lkm_init_boot_file.exists() and lkm_vbmeta_file.exists()
    
    unroot_mode: Optional[str] = None
    
    if gki_exists and lkm_exists:
        os.system('cls')
        utils.ui.echo("\n  " + "=" * 78)
        utils.ui.echo(get_string("act_unroot_menu_title"))
        utils.ui.echo("  " + "=" * 78 + "\n")
        utils.ui.echo(get_string("act_unroot_menu_1_lkm"))
        utils.ui.echo(get_string("act_unroot_menu_2_gki"))
        utils.ui.echo("\n" + get_string("act_unroot_menu_m"))
        utils.ui.echo("\n  " + "=" * 78 + "\n")
        
        while unroot_mode is None:
            choice = utils.ui.prompt(get_string("act_unroot_menu_prompt")).strip().lower()
            if choice == "1":
                unroot_mode = "lkm"
            elif choice == "2":
                unroot_mode = "gki"
            elif choice == "m":
                utils.ui.echo(get_string("act_op_cancel"))
                return
            else:
                utils.ui.echo(get_string("act_unroot_menu_invalid"))
                
    elif lkm_exists:
        utils.ui.echo(get_string("act_unroot_lkm_detected"))
        unroot_mode = "lkm"
    elif gki_exists:
        utils.ui.echo(get_string("act_unroot_gki_detected"))
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
            utils.ui.echo(get_string("act_unroot_lkm_detected"))
        else:
            unroot_mode = "gki"
            utils.ui.echo(get_string("act_unroot_gki_detected"))

    utils.ui.echo(get_string("act_unroot_step1"))

    edl.ensure_edl_requirements()

    utils.ui.echo(get_string("act_unroot_step3"))
    if not dev.skip_adb:
        dev.wait_for_adb()
    
    active_slot = detect_active_slot_robust(dev)
    suffix = active_slot if active_slot else ""

    port = dev.setup_edl_connection()

    try:
        dev.load_firehose_programmer_with_stability(const.EDL_LOADER_FILE, port)
    except Exception as e:
        utils.ui.echo(get_string("act_warn_prog_load").format(e=e))

    try:
        if unroot_mode == "lkm":
            target_init_boot = f"init_boot{suffix}"
            target_vbmeta = f"vbmeta{suffix}"
            utils.ui.echo(get_string("act_unroot_step4_lkm"))

            edl.flash_partition_target(dev, port, target_init_boot, lkm_init_boot_file)
            utils.ui.echo(get_string("act_flash_stock_init_boot_ok").format(part=target_init_boot))

            edl.flash_partition_target(dev, port, target_vbmeta, lkm_vbmeta_file)
            utils.ui.echo(get_string("act_flash_stock_vbmeta_ok").format(part=target_vbmeta))
            
        elif unroot_mode == "gki":
            target_boot = f"boot{suffix}"
            utils.ui.echo(get_string("act_unroot_step4_gki").format(part=target_boot))
            
            edl.flash_partition_target(dev, port, target_boot, gki_boot_file)
            utils.ui.echo(get_string("act_flash_stock_boot_ok").format(part=target_boot))
        
        utils.ui.echo(get_string("act_reset_sys"))
        dev.edl_reset(port)
        
    except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
        utils.ui.error(get_string("act_err_edl_write").format(e=e))
        raise

    utils.ui.echo(get_string("act_unroot_finish"))

def sign_and_flash_twrp(dev: device.DeviceController) -> None:
    utils.ui.echo(get_string("act_start_rec_flash"))

    twrp_name = const.FN_TWRP
    out_dir = const.OUTPUT_TWRP_DIR
    
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(exist_ok=True)

    utils.check_dependencies()
    edl.ensure_edl_requirements()

    utils.ui.echo(get_string("act_wait_image"))
    prompt = get_string("act_prompt_twrp").format(dir=const.IMAGE_DIR.name)
    utils.wait_for_files(const.IMAGE_DIR, [twrp_name], prompt)
    
    twrp_src = const.IMAGE_DIR / twrp_name

    utils.ui.echo(get_string("act_root_step1"))
    if not dev.skip_adb:
        dev.wait_for_adb()
    
    active_slot = detect_active_slot_robust(dev)
    suffix = active_slot if active_slot else ""
    target_partition = f"recovery{suffix}"

    utils.ui.echo(get_string("act_root_step2"))
    port = dev.setup_edl_connection()
    try:
        dev.load_firehose_programmer_with_stability(const.EDL_LOADER_FILE, port)
    except Exception as e:
        utils.ui.echo(get_string("act_warn_prog_load").format(e=e))

    with utils.temporary_workspace(const.WORK_DIR):
        dumped_recovery = const.WORK_DIR / f"recovery{suffix}.img"

        utils.ui.echo(get_string("act_dump_recovery").format(part=target_partition))
        try:
            params = ensure_params_or_fail(target_partition)
            dev.edl_read_partition(
                port=port,
                output_filename=str(dumped_recovery),
                lun=params['lun'],
                start_sector=params['start_sector'],
                num_sectors=params['num_sectors']
            )
        except Exception as e:
            utils.ui.error(get_string("act_err_dump").format(part=target_partition, e=e))
            raise

        backup_recovery = const.BACKUP_DIR / f"recovery{suffix}.img"
        const.BACKUP_DIR.mkdir(exist_ok=True)
        shutil.copy(dumped_recovery, backup_recovery)
        utils.ui.echo(get_string("act_backup_recovery_ok"))

        dev.edl_reset(port)

        utils.ui.echo(get_string("act_sign_twrp_start"))
        
        rec_info = extract_image_avb_info(dumped_recovery)
        
        pubkey = rec_info.get('pubkey_sha1')
        key_file = const.KEY_MAP.get(pubkey)
        
        if not key_file:
             utils.ui.error(get_string("img_err_boot_key_mismatch").format(key=pubkey))
             raise KeyError(f"Unknown key: {pubkey}")

        final_twrp = out_dir / twrp_name
        shutil.copy(twrp_src, final_twrp)
        
        subprocess.run(
            [str(const.PYTHON_EXE), str(const.AVBTOOL_PY), "erase_footer", "--image", str(final_twrp)],
            capture_output=True
        )
        
        _apply_hash_footer(
            image_path=final_twrp,
            image_info=rec_info,
            key_file=key_file
        )
        utils.ui.echo(get_string("act_sign_twrp_ok"))

        utils.ui.echo(get_string("act_reboot_edl_flash"))
        if not dev.skip_adb:
            dev.wait_for_adb()
            port = dev.setup_edl_connection()
        else:
             port = dev.wait_for_edl()

        try:
            dev.load_firehose_programmer_with_stability(const.EDL_LOADER_FILE, port)
        except Exception:
            pass

        utils.ui.echo(get_string("act_flash_twrp").format(part=target_partition))
        edl.flash_partition_target(dev, port, target_partition, final_twrp)

        utils.ui.echo(get_string("act_reset_sys"))
        dev.edl_reset(port)

    utils.ui.echo(get_string("act_success"))