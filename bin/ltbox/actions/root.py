import os
import shutil
import subprocess
import zipfile
from abc import ABC, abstractmethod
from pathlib import Path
from typing import Optional, List, Dict

from . import edl
from .. import constants as const
from .. import utils, device, downloader
from ..downloader import ensure_magiskboot
from ..errors import ToolError
from ..partition import ensure_params_or_fail
from .system import detect_active_slot_robust
from ..patch.root import patch_boot_with_root_algo
from ..patch.avb import process_boot_image_avb, rebuild_vbmeta_with_chained_images
from ..i18n import get_string
from ..main import TerminalMenu

class RootStrategy(ABC):
    @property
    @abstractmethod
    def image_name(self) -> str: pass

    @property
    @abstractmethod
    def backup_name(self) -> str: pass

    @property
    @abstractmethod
    def output_dir(self) -> Path: pass

    @property
    @abstractmethod
    def backup_dir(self) -> Path: pass

    @property
    @abstractmethod
    def required_files(self) -> List[str]: pass

    @property
    @abstractmethod
    def log_output_dir_name(self) -> str: pass

    @abstractmethod
    def get_partition_map(self, suffix: str) -> Dict[str, str]: pass

    @abstractmethod
    def patch(self, work_dir: Path, dev: Optional[device.DeviceController] = None, lkm_kernel_version: Optional[str] = None) -> Path: pass

    @abstractmethod
    def finalize_patch(self, patched_boot: Path, output_dir: Path, backup_source_dir: Path) -> Path: pass

class GkiRootStrategy(RootStrategy):
    @property
    def image_name(self) -> str:
        return const.FN_BOOT

    @property
    def backup_name(self) -> str:
        return const.FN_BOOT_BAK

    @property
    def output_dir(self) -> Path:
        return const.OUTPUT_ROOT_DIR

    @property
    def backup_dir(self) -> Path:
        return const.BACKUP_BOOT_DIR

    @property
    def required_files(self) -> List[str]:
        return [self.image_name]

    @property
    def log_output_dir_name(self) -> str:
        return const.OUTPUT_ROOT_DIR.name

    def get_partition_map(self, suffix: str) -> Dict[str, str]:
        return {
            "main": f"boot{suffix}",
            "vbmeta": ""
        }

    def patch(self, work_dir: Path, dev: Optional[device.DeviceController] = None, lkm_kernel_version: Optional[str] = None) -> Path:
        magiskboot_exe = utils.get_platform_executable("magiskboot")
        ensure_magiskboot()
        
        downloader.download_gki_tools(gki=True)
        
        return patch_boot_with_root_algo(work_dir, magiskboot_exe, dev=None, gki=True)

    def finalize_patch(self, patched_boot: Path, output_dir: Path, backup_source_dir: Path) -> Path:
        process_boot_image_avb(patched_boot, gki=True)
        final_boot = output_dir / self.image_name
        shutil.move(patched_boot, final_boot)
        return final_boot

class LkmRootStrategy(RootStrategy):
    def __init__(self, root_type: str = "ksu"):
        self.root_type = root_type

    @property
    def image_name(self) -> str:
        return const.FN_INIT_BOOT

    @property
    def backup_name(self) -> str:
        return const.FN_INIT_BOOT_BAK

    @property
    def output_dir(self) -> Path:
        return const.OUTPUT_ROOT_LKM_DIR

    @property
    def backup_dir(self) -> Path:
        return const.BACKUP_INIT_BOOT_DIR

    @property
    def required_files(self) -> List[str]:
        return [self.image_name, const.FN_VBMETA]

    @property
    def log_output_dir_name(self) -> str:
        return const.OUTPUT_ROOT_LKM_DIR.name

    def get_partition_map(self, suffix: str) -> Dict[str, str]:
        return {
            "main": f"init_boot{suffix}",
            "vbmeta": f"vbmeta{suffix}"
        }
    
    def _cleanup_manager_apk(self):
        manager_apk = const.TOOLS_DIR / "manager.apk"
        if manager_apk.exists():
            manager_apk.unlink()

    def _get_mapped_kernel_name(self, kernel_version: str) -> Optional[str]:
        if not kernel_version: return None
        major_minor = ".".join(kernel_version.split(".")[:2])
        mapping = {
            "5.10": "android12-5.10", "5.15": "android13-5.15",
            "6.1":  "android14-6.1",  "6.6":  "android15-6.6",
            "6.12": "android16-6.12"
        }
        return mapping.get(major_minor)

    def _prompt_workflow(self, root_name: str, default_id: str) -> str:
        msg_enter = get_string("prompt_workflow_id").replace("{name}", root_name)
        msg_default = get_string("prompt_workflow_default").replace("{id}", default_id)
        
        utils.ui.echo("-" * 60)
        utils.ui.echo(msg_enter)
        utils.ui.echo(msg_default)
        utils.ui.echo("-" * 60)
        
        val = input("Input > ").strip()
        if not val:
            return default_id
        return val

    def _download_nightly(self, root_name, repo, default_workflow, manager_zip, kernel_version, work_dir) -> bool:
        mapped_name = self._get_mapped_kernel_name(kernel_version)
        if not mapped_name:
            utils.ui.error(get_string("err_sukisu_kernel_map_not_found").format(ver=kernel_version))
            return False

        while True:
            workflow_id = self._prompt_workflow(root_name, default_workflow)
            try:
                temp_dl_dir = const.TOOLS_DIR / "dl_temp"
                if temp_dl_dir.exists(): shutil.rmtree(temp_dl_dir)
                temp_dl_dir.mkdir(exist_ok=True)

                downloader.download_nightly_artifacts(
                    repo=repo, workflow_id=workflow_id,
                    manager_name=manager_zip, mapped_name=mapped_name,
                    target_dir=temp_dl_dir
                )

                mgr_zip_path = temp_dl_dir / manager_zip
                apk_found = False
                if mgr_zip_path.exists():
                    with zipfile.ZipFile(mgr_zip_path, 'r') as zf:
                        for name in zf.namelist():
                            if name.endswith(".apk"):
                                with zf.open(name) as src, open(const.TOOLS_DIR / "manager.apk", "wb") as dst:
                                    shutil.copyfileobj(src, dst)
                                apk_found = True
                                break
                
                if not apk_found:
                    raise ToolError("Manager APK not found in zip.")

                lkm_zip = temp_dl_dir / "lkm.zip"
                ko_found = False
                if lkm_zip.exists():
                    with zipfile.ZipFile(lkm_zip, 'r') as zf:
                        for name in zf.namelist():
                            if name.endswith("kernelsu.ko"):
                                with zf.open(name) as src, open(work_dir / "kernelsu.ko", "wb") as dst:
                                    shutil.copyfileobj(src, dst)
                                ko_found = True
                                break
                
                if not ko_found:
                    raise ToolError("kernelsu.ko not found in zip.")
                
                shutil.copy(temp_dl_dir / "ksuinit", work_dir / "init")
                
                shutil.rmtree(temp_dl_dir)
                return True

            except Exception as e:
                utils.ui.error(f"{e}")
                utils.ui.error(get_string("err_download_workflow"))
                
                choice = utils.ui.prompt(get_string("press_enter_to_continue"))
                return False

    def patch(self, work_dir: Path, dev: Optional[device.DeviceController] = None, lkm_kernel_version: Optional[str] = None) -> Path:
        self._cleanup_manager_apk()
        magiskboot_exe = utils.get_platform_executable("magiskboot")
        ensure_magiskboot()
        settings = const.load_settings_raw()

        if self.root_type == "sukisu":
            conf = settings.get("sukisu-ultra", {})
            success = self._download_nightly(
                "SukiSU Ultra", conf.get("repo"), conf.get("workflow"),
                conf.get("manager"), lkm_kernel_version, work_dir
            )
            if not success: return None
            
        else:
            menu = TerminalMenu(get_string("menu_root_subtype_title"))
            menu.add_option("1", get_string("menu_root_subtype_release"))
            menu.add_option("2", get_string("menu_root_subtype_nightly"))
            
            while True:
                choice = menu.ask(get_string("menu_root_subtype_prompt"), get_string("menu_invalid"))
                if choice == "1":
                    downloader.download_ksu_manager_release(const.TOOLS_DIR)
                    downloader.download_ksuinit_release(work_dir / "init")
                    downloader.get_lkm_kernel_release(work_dir / "kernelsu.ko", lkm_kernel_version)
                    break
                elif choice == "2":
                    conf = settings.get("kernelsu-next", {})
                    success = self._download_nightly(
                        "KernelSU Next", conf.get("repo"), conf.get("nightly_workflow"),
                        conf.get("nightly_manager"), lkm_kernel_version, work_dir
                    )
                    if not success: return None
                    break

        return patch_boot_with_root_algo(
            work_dir, magiskboot_exe, dev, gki=False, 
            lkm_kernel_version=lkm_kernel_version, 
            root_type=self.root_type,
            skip_lkm_download=True
        )

    def finalize_patch(self, patched_boot: Path, output_dir: Path, backup_source_dir: Path) -> Path:
        process_boot_image_avb(patched_boot, gki=False)
        
        vbmeta_bak = backup_source_dir / const.FN_VBMETA_BAK
        patched_vbmeta_path = const.BASE_DIR / const.FN_VBMETA_ROOT
        
        rebuild_vbmeta_with_chained_images(
            output_path=patched_vbmeta_path,
            original_vbmeta_path=vbmeta_bak,
            chained_images=[patched_boot]
        )
        
        final_boot = output_dir / self.image_name
        shutil.move(patched_boot, final_boot)
        
        if patched_vbmeta_path.exists():
            shutil.move(patched_vbmeta_path, output_dir / const.FN_VBMETA)
            
        return final_boot

def patch_root_image_file(gki: bool = False, root_type: str = "ksu") -> None:
    strategy = GkiRootStrategy() if gki else LkmRootStrategy(root_type)
    
    utils.ui.echo(get_string("act_clean_root_out").format(dir=strategy.log_output_dir_name))
    if strategy.output_dir.exists():
        shutil.rmtree(strategy.output_dir)
    strategy.output_dir.mkdir(exist_ok=True)
    utils.ui.echo("")
    
    utils.check_dependencies()

    wait_msg = get_string("act_wait_boot") if gki else get_string("act_wait_init_boot")
    utils.ui.echo(wait_msg)
    const.IMAGE_DIR.mkdir(exist_ok=True)

    prompt = get_string("act_prompt_boot").format(name=const.IMAGE_DIR.name)
    if not gki:
        prompt = prompt.replace(f"'{const.FN_BOOT}'", f"'{const.FN_INIT_BOOT}' and '{const.FN_VBMETA}'")

    utils.wait_for_files(const.IMAGE_DIR, strategy.required_files, prompt)

    for fname in strategy.required_files:
        src = const.IMAGE_DIR / fname
        dst = const.BASE_DIR / fname
        try:
            shutil.copy(src, dst)
            utils.ui.echo(get_string("act_copy_boot").format(name=src.name))
        except (IOError, OSError) as e:
            utils.ui.error(get_string("act_err_copy_boot").format(name=src.name, e=e))
            raise ToolError(get_string("act_err_copy_boot").format(name=src.name, e=e))

    if not (const.BASE_DIR / strategy.image_name).exists():
        msg = get_string("act_err_boot_missing") if gki else get_string("act_err_init_boot_missing")
        utils.ui.echo(msg)
        raise ToolError(msg)

    utils.ui.echo(get_string("act_backup_boot"))
    shutil.copy(const.BASE_DIR / strategy.image_name, const.BASE_DIR / strategy.backup_name)
    if not gki:
        shutil.copy(const.BASE_DIR / const.FN_VBMETA, const.BASE_DIR / const.FN_VBMETA_BAK)

    patched_boot_path = None
    with utils.temporary_workspace(const.WORK_DIR):
        shutil.copy(const.BASE_DIR / strategy.image_name, const.WORK_DIR / strategy.image_name)
        (const.BASE_DIR / strategy.image_name).unlink()
        
        if not gki:
            (const.BASE_DIR / const.FN_VBMETA).unlink()
        
        lkm_kernel_version = None
        if not gki:
             utils.ui.echo(get_string("err_req_kernel_ver_lkm"))
             lkm_kernel_version = input("Enter Kernel Version (e.g. 5.15.100): ").strip()
             if not lkm_kernel_version:
                 utils.ui.error("Kernel version required.")
                 return

        patched_boot_path = strategy.patch(const.WORK_DIR, dev=None, lkm_kernel_version=lkm_kernel_version)

    if patched_boot_path and patched_boot_path.exists():
        utils.ui.echo(get_string("act_finalize_root"))
        
        strategy.finalize_patch(patched_boot_path, strategy.output_dir, const.BASE_DIR)
        utils.ui.echo("")

        utils.ui.echo(get_string("act_move_root_backup").format(dir=const.BACKUP_DIR.name))
        const.BACKUP_DIR.mkdir(exist_ok=True)
        for bak_file in const.BASE_DIR.glob("*.bak.img"):
            shutil.move(bak_file, const.BACKUP_DIR / bak_file.name)
        utils.ui.echo("")

        utils.ui.echo("  " + "=" * 78)
        utils.ui.echo(get_string("act_success"))
        
        success_msg = get_string("act_root_saved").format(dir=strategy.log_output_dir_name)
        if not gki:
             success_msg = get_string("act_root_saved_lkm").format(dir=strategy.log_output_dir_name)
             
        utils.ui.echo(success_msg)
        if not gki:
            utils.ui.echo(get_string("act_root_saved_vbmeta_lkm").format(name=const.FN_VBMETA, dir=strategy.log_output_dir_name))
        
        utils.ui.echo("\n" + get_string("act_root_manual_flash_notice"))
        utils.ui.echo("  " + "=" * 78)
    else:
        fail_msg = get_string("act_err_root_fail") if gki else get_string("act_err_root_fail_lkm")
        utils.ui.error(fail_msg)

def _prepare_root_env(strategy: RootStrategy):
    utils.ui.echo(get_string("act_start_root"))
    
    if strategy.output_dir.exists():
        shutil.rmtree(strategy.output_dir)
    strategy.output_dir.mkdir(exist_ok=True)
    strategy.backup_dir.mkdir(exist_ok=True)

    utils.check_dependencies()
    edl.ensure_edl_requirements()
    ensure_magiskboot()

def _get_lkm_kernel_version(dev: device.DeviceController, gki: bool) -> Optional[str]:
    if not gki:
        if not dev.skip_adb:
            try:
                return dev.adb.get_kernel_version()
            except Exception as e:
                utils.ui.error(get_string("act_root_warn_lkm_kver_fail").format(e=e))
                utils.ui.error(get_string("act_root_warn_lkm_kver_retry"))
        else:
            utils.ui.error(get_string("act_root_err_lkm_skip_adb"))
            raise ToolError(get_string("act_root_err_lkm_skip_adb_exc"))
    return None

def _dump_partition_to_workspace(dev: device.DeviceController, port: str, label: str, output_path: Path):
    params = ensure_params_or_fail(label)
    utils.ui.echo(get_string("act_found_dump_info").format(xml=params['source_xml'], lun=params['lun'], start=params['start_sector']))
    dev.edl.read_partition(
        port=port,
        output_filename=str(output_path),
        lun=params['lun'],
        start_sector=params['start_sector'],
        num_sectors=params['num_sectors']
    )
    if params.get('size_in_kb'):
        expected = int(float(params['size_in_kb']) * 1024)
        actual = output_path.stat().st_size
        if expected != actual:
            raise RuntimeError(get_string("act_err_dump_mismatch").format(part=label, expected=expected, actual=actual))

def _dump_and_generate_root_image(dev: device.DeviceController, port: str, strategy: RootStrategy, 
                                  partition_map: Dict[str, str], gki: bool, lkm_kernel_version: Optional[str]) -> Path:
    
    main_partition = partition_map["main"]
    step3_msg = get_string("act_root_step3") if gki else get_string("act_root_step3_init_boot")
    utils.ui.echo(step3_msg.format(part=main_partition))

    with utils.temporary_workspace(const.WORKING_BOOT_DIR):
        dumped_main = const.WORKING_BOOT_DIR / strategy.image_name
        backup_main = strategy.backup_dir / strategy.image_name
        base_main_bak = const.BASE_DIR / strategy.backup_name
        
        try:
            _dump_partition_to_workspace(dev, port, main_partition, dumped_main)

            if not gki:
                vbmeta_partition = partition_map["vbmeta"]
                dumped_vbmeta = const.WORKING_BOOT_DIR / const.FN_VBMETA
                _dump_partition_to_workspace(dev, port, vbmeta_partition, dumped_vbmeta)

            read_ok_msg = get_string("act_read_boot_ok") if gki else get_string("act_read_init_boot_ok")
            utils.ui.echo(read_ok_msg.format(part=main_partition, file=dumped_main))

        except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
            utils.ui.error(get_string("act_err_dump").format(part=main_partition, e=e))
            raise

        utils.ui.echo(get_string("act_backup_boot_root").format(dir=strategy.backup_dir.name))
        shutil.copy(dumped_main, backup_main)
        utils.ui.echo(get_string("act_temp_backup_avb"))
        shutil.copy(dumped_main, base_main_bak)
        
        if not gki:
            shutil.copy(const.WORKING_BOOT_DIR / const.FN_VBMETA, strategy.backup_dir / const.FN_VBMETA)
            shutil.copy(const.WORKING_BOOT_DIR / const.FN_VBMETA, const.BASE_DIR / const.FN_VBMETA_BAK)
        
        utils.ui.echo(get_string("act_backups_done"))
        utils.ui.echo(get_string("act_dump_reset"))
        dev.edl.reset(port)
        
        step4_msg = get_string("act_root_step4") if gki else get_string("act_root_step4_init_boot")
        utils.ui.echo(step4_msg)

        try:
            patched_boot_path = strategy.patch(const.WORKING_BOOT_DIR, dev, lkm_kernel_version)
            if not (patched_boot_path and patched_boot_path.exists()):
                raise ToolError(get_string("act_err_root_fail"))

            utils.ui.echo(get_string("act_root_step5"))
            final_boot = strategy.finalize_patch(patched_boot_path, strategy.output_dir, const.BASE_DIR)
            utils.ui.echo(get_string("act_patched_boot_saved").format(dir=final_boot.parent.name))
        except Exception as e:
            if isinstance(e, ToolError):
                utils.ui.error(str(e))
            else:
                utils.ui.error(get_string("act_err_avb_footer").format(e=e))
            base_main_bak.unlink(missing_ok=True)
            if not gki: (const.BASE_DIR / const.FN_VBMETA_BAK).unlink(missing_ok=True)
            raise

        base_main_bak.unlink(missing_ok=True)
        if not gki: (const.BASE_DIR / const.FN_VBMETA_BAK).unlink(missing_ok=True)

        return strategy.output_dir / strategy.image_name

def _flash_root_image(dev: device.DeviceController, strategy: RootStrategy, partition_map: Dict[str, str], gki: bool):
    main_partition = partition_map["main"]
    step6_msg = get_string("act_root_step6") if gki else get_string("act_root_step6_init_boot")
    utils.ui.echo(step6_msg.format(part=main_partition))

    if not dev.skip_adb:
        utils.ui.echo(get_string("act_wait_sys_adb"))
        dev.adb.wait_for_device()
        utils.ui.echo(get_string("act_reboot_edl_flash"))
        port = dev.setup_edl_connection()
    else:
        utils.ui.echo(get_string("act_skip_adb_on"))
        utils.ui.echo(get_string("act_manual_edl_now"))
        port = dev.edl.wait_for_device()

    try:
        dev.edl.load_programmer_safe(port, const.EDL_LOADER_FILE)
    except Exception as e:
        utils.ui.echo(get_string("act_warn_prog_load").format(e=e))

    try:
        final_boot_path = strategy.output_dir / strategy.image_name
        edl.flash_partition_target(dev, port, main_partition, final_boot_path)
        
        flash_ok_msg = get_string("act_flash_boot_ok") if gki else get_string("act_flash_init_boot_ok")
        utils.ui.echo(flash_ok_msg.format(part=main_partition))

        if not gki:
            final_vbmeta_path = strategy.output_dir / const.FN_VBMETA
            vbmeta_part = partition_map["vbmeta"]
            edl.flash_partition_target(dev, port, vbmeta_part, final_vbmeta_path)
            utils.ui.echo(get_string("act_flash_vbmeta_ok").format(part=vbmeta_part))

        utils.ui.echo(get_string("act_reset_sys"))
        dev.edl.reset(port)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        utils.ui.error(get_string("act_err_edl_write").format(e=e))
        raise

def root_device(dev: device.DeviceController, gki: bool = False, root_type: str = "ksu") -> None:
    strategy = GkiRootStrategy() if gki else LkmRootStrategy(root_type)

    _prepare_root_env(strategy)
    
    utils.ui.echo(get_string("act_root_step1"))
    if not dev.skip_adb:
        dev.adb.wait_for_device()

    active_slot = detect_active_slot_robust(dev)
    suffix = active_slot if active_slot else ""
    
    lkm_kernel_version = _get_lkm_kernel_version(dev, gki)

    partition_map = strategy.get_partition_map(suffix)
    main_partition = partition_map["main"]
    
    if active_slot:
        utils.ui.echo(get_string("act_slot_confirmed").format(slot=active_slot))
    else:
        utils.ui.echo(get_string("act_warn_root_slot"))
        main_partition = strategy.image_name.replace(".img", "")
        if gki: partition_map["main"] = "boot"
        else: partition_map["main"] = "init_boot"

    utils.ui.echo(get_string("act_root_step2"))
    port = dev.setup_edl_connection()
    try:
        dev.edl.load_programmer_safe(port, const.EDL_LOADER_FILE)
    except Exception as e:
        utils.ui.echo(get_string("act_warn_prog_load").format(e=e))

    _dump_and_generate_root_image(dev, port, strategy, partition_map, gki, lkm_kernel_version)

    _flash_root_image(dev, strategy, partition_map, gki)

    utils.ui.echo(get_string("act_root_finish"))

def unroot_device(dev: device.DeviceController) -> None:
    utils.ui.echo(get_string("act_start_unroot"))
    
    gki_strategy = GkiRootStrategy()
    lkm_strategy = LkmRootStrategy()
    
    gki_boot_file = gki_strategy.backup_dir / gki_strategy.image_name
    lkm_init_boot_file = lkm_strategy.backup_dir / lkm_strategy.image_name
    lkm_vbmeta_file = lkm_strategy.backup_dir / const.FN_VBMETA
    
    gki_exists = gki_boot_file.exists()
    lkm_exists = lkm_init_boot_file.exists() and lkm_vbmeta_file.exists()
    
    selected_strategy = None
    
    if gki_exists and lkm_exists:
        os.system('cls')
        utils.ui.echo("\n  " + "=" * 78)
        utils.ui.echo(get_string("act_unroot_menu_title"))
        utils.ui.echo("  " + "=" * 78 + "\n")
        utils.ui.echo(get_string("act_unroot_menu_1_lkm"))
        utils.ui.echo(get_string("act_unroot_menu_2_gki"))
        utils.ui.echo("\n" + get_string("act_unroot_menu_m"))
        utils.ui.echo("\n  " + "=" * 78 + "\n")
        
        while selected_strategy is None:
            choice = utils.ui.prompt(get_string("act_unroot_menu_prompt")).strip().lower()
            if choice == "1":
                selected_strategy = lkm_strategy
            elif choice == "2":
                selected_strategy = gki_strategy
            elif choice == "m":
                utils.ui.echo(get_string("act_op_cancel"))
                return
            else:
                utils.ui.echo(get_string("act_unroot_menu_invalid"))
                
    elif lkm_exists:
        utils.ui.echo(get_string("act_unroot_lkm_detected"))
        selected_strategy = lkm_strategy
    elif gki_exists:
        utils.ui.echo(get_string("act_unroot_gki_detected"))
        selected_strategy = gki_strategy
    else:
        prompt = get_string("act_unroot_prompt_all").format(
            lkm_dir=lkm_strategy.backup_dir.name, 
            gki_dir=gki_strategy.backup_dir.name
        )
        
        def check_for_unroot_files(p: Path, f: Optional[list]) -> bool:
            return gki_boot_file.exists() or (lkm_init_boot_file.exists() and lkm_vbmeta_file.exists())
        
        utils._wait_for_resource(const.BASE_DIR, check_for_unroot_files, prompt, None)
        
        if lkm_init_boot_file.exists() and lkm_vbmeta_file.exists():
            selected_strategy = lkm_strategy
            utils.ui.echo(get_string("act_unroot_lkm_detected"))
        else:
            selected_strategy = gki_strategy
            utils.ui.echo(get_string("act_unroot_gki_detected"))

    utils.ui.echo(get_string("act_unroot_step1"))
    edl.ensure_edl_requirements()
    utils.ui.echo(get_string("act_unroot_step3"))
    
    if not dev.skip_adb:
        dev.adb.wait_for_device()
    
    active_slot = detect_active_slot_robust(dev)
    suffix = active_slot if active_slot else ""
    port = dev.setup_edl_connection()

    try:
        dev.edl.load_programmer_safe(port, const.EDL_LOADER_FILE)
    except Exception as e:
        utils.ui.echo(get_string("act_warn_prog_load").format(e=e))

    try:
        partition_map = selected_strategy.get_partition_map(suffix)
        
        if isinstance(selected_strategy, LkmRootStrategy):
            utils.ui.echo(get_string("act_unroot_step4_lkm"))
            
            target_init_boot = partition_map["main"]
            edl.flash_partition_target(dev, port, target_init_boot, lkm_init_boot_file)
            utils.ui.echo(get_string("act_flash_stock_init_boot_ok").format(part=target_init_boot))

            target_vbmeta = partition_map["vbmeta"]
            edl.flash_partition_target(dev, port, target_vbmeta, lkm_vbmeta_file)
            utils.ui.echo(get_string("act_flash_stock_vbmeta_ok").format(part=target_vbmeta))
            
        elif isinstance(selected_strategy, GkiRootStrategy):
            target_boot = partition_map["main"]
            utils.ui.echo(get_string("act_unroot_step4_gki").format(part=target_boot))
            
            edl.flash_partition_target(dev, port, target_boot, gki_boot_file)
            utils.ui.echo(get_string("act_flash_stock_boot_ok").format(part=target_boot))
        
        utils.ui.echo(get_string("act_reset_sys"))
        dev.edl.reset(port)
        
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
        dev.adb.wait_for_device()
    
    active_slot = detect_active_slot_robust(dev)
    suffix = active_slot if active_slot else ""
    target_partition = f"recovery{suffix}"

    utils.ui.echo(get_string("act_root_step2"))
    port = dev.setup_edl_connection()
    try:
        dev.edl.load_programmer_safe(port, const.EDL_LOADER_FILE)
    except Exception as e:
        utils.ui.echo(get_string("act_warn_prog_load").format(e=e))

    with utils.temporary_workspace(const.WORK_DIR):
        dumped_recovery = const.WORK_DIR / f"recovery{suffix}.img"

        utils.ui.echo(get_string("act_dump_recovery").format(part=target_partition))
        try:
            params = ensure_params_or_fail(target_partition)
            dev.edl.read_partition(
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

        dev.edl.reset(port)

        utils.ui.echo(get_string("act_sign_twrp_start"))
        
        from ..patch.avb import extract_image_avb_info, _apply_hash_footer
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
            dev.adb.wait_for_device()
            port = dev.setup_edl_connection()
        else:
             port = dev.edl.wait_for_device()

        try:
            dev.edl.load_programmer_safe(port, const.EDL_LOADER_FILE)
        except Exception:
            pass

        utils.ui.echo(get_string("act_flash_twrp").format(part=target_partition))
        edl.flash_partition_target(dev, port, target_partition, final_twrp)

        utils.ui.echo(get_string("act_reset_sys"))
        dev.edl.reset(port)

    utils.ui.echo(get_string("act_success"))