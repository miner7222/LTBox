import json
import os
import platform
import subprocess
import sys
import webbrowser
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Tuple

from . import downloader, i18n, menu_data, utils
from .i18n import get_string
from .logger import logging_context
from .menu import TerminalMenu, select_menu_action
from .menu_data import (
    get_advanced_menu_data,
    get_main_menu_data,
    get_settings_menu_data,
)
from .utils import ui

APP_DIR = Path(__file__).parent.resolve()
BASE_DIR = APP_DIR.parent
PYTHON_EXE = BASE_DIR / "python3" / "python.exe"
SETTINGS_FILE = APP_DIR / "settings.json"

try:
    from .errors import LTBoxError, ToolError
except ImportError:
    print(get_string("err_import_critical"), file=sys.stderr)
    print(get_string("err_ensure_errors"), file=sys.stderr)
    input(get_string("press_enter_to_exit"))
    sys.exit(1)

# --- Command Registry ---


@dataclass(frozen=True)
class CommandSpec:
    func: Callable
    title: str
    require_dev: bool = True
    default_kwargs: Dict[str, Any] = field(default_factory=dict)

    def __getitem__(self, key: str) -> Any:
        try:
            return getattr(self, key)
        except AttributeError as exc:
            raise KeyError(key) from exc


class CommandRegistry:
    def __init__(self):
        self._commands: Dict[str, CommandSpec] = {}

    def register(
        self, name: str, title: str, require_dev: bool = True, **default_kwargs
    ):
        def decorator(func: Callable):
            self._commands[name] = CommandSpec(
                func=func,
                title=title,
                require_dev=require_dev,
                default_kwargs=default_kwargs,
            )
            return func

        return decorator

    def add(
        self,
        name: str,
        func: Callable,
        title: str,
        require_dev: bool = True,
        **default_kwargs,
    ):
        self.register(name, title, require_dev, **default_kwargs)(func)

    def get(self, name: str) -> Optional[CommandSpec]:
        return self._commands.get(name)


# --- UI Helper Class ---


def _format_command_failure_messages(
    error: subprocess.CalledProcessError,
) -> List[str]:
    messages = [
        get_string("err_cmd_failed").format(
            cmd=" ".join(error.cmd) if isinstance(error.cmd, list) else error.cmd
        )
    ]
    if error.stdout:
        messages.append(f"{get_string('err_cmd_stdout_header')}\n{error.stdout}")
    if error.stderr:
        messages.append(f"{get_string('err_cmd_stderr_header')}\n{error.stderr}")
    return messages


# --- Settings & Init ---


def _load_settings() -> Dict[str, Any]:
    if SETTINGS_FILE.exists():
        try:
            with open(SETTINGS_FILE, "r", encoding="utf-8") as f:
                return json.load(f)
        except Exception:
            return {}
    return {}


def _save_settings(data: Dict[str, Any]) -> None:
    try:
        current_settings = _load_settings()
        current_settings.update(data)
        with open(SETTINGS_FILE, "w", encoding="utf-8") as f:
            json.dump(current_settings, f, indent=2)
    except Exception as e:
        print(f"Warning: Failed to save settings: {e}", file=sys.stderr)


def _read_current_version() -> str:
    config_file = APP_DIR / "config.json"
    if config_file.exists():
        try:
            with open(config_file, "r", encoding="utf-8") as f:
                config_data = json.load(f)
                return config_data.get("version", "v0.0.0")
        except Exception:
            return "v0.0.0"
    return "v0.0.0"


def _get_latest_version(
    current_version: str,
) -> Tuple[Optional[str], Optional[str], Optional[str]]:
    try:
        latest_release, latest_prerelease = utils.get_latest_release_versions(
            "miner7222", "LTBox"
        )
        latest_version = None

        if latest_release and utils.is_update_available(
            current_version, latest_release
        ):
            latest_version = latest_release
        elif latest_release and utils.is_update_available(
            latest_release, current_version
        ):
            if latest_prerelease and utils.is_update_available(
                current_version, latest_prerelease
            ):
                latest_version = latest_prerelease
        elif latest_release is None and latest_prerelease:
            if utils.is_update_available(current_version, latest_prerelease):
                latest_version = latest_prerelease

        return latest_version, latest_release, latest_prerelease
    except Exception:
        return None, None, None


def _abort_platform_check(messages: List[str]) -> None:
    for message in messages:
        print(message, file=sys.stderr)
    print(get_string("err_aborting"), file=sys.stderr)
    input(get_string("press_enter_to_exit"))
    sys.exit(1)


def _check_platform():
    if platform.system() != "Windows":
        _abort_platform_check(
            [
                get_string("err_fatal_windows"),
                get_string("err_current_platform").format(platform=platform.system()),
            ]
        )

    if platform.machine() != "AMD64":
        _abort_platform_check(
            [
                get_string("err_fatal_amd64"),
                get_string("err_current_arch").format(arch=platform.machine()),
                get_string("err_arch_unsupported"),
            ]
        )


def setup_console():
    try:
        import ctypes

        if sys.platform == "win32":
            ctypes.windll.kernel32.SetConsoleTitleW("LTBox")

        sys.stdout.write("\x1b[8;40;80t")
        sys.stdout.flush()

        os.system("mode con: cols=80 lines=40")

    except Exception as e:
        print(get_string("warn_set_console_title").format(e=e), file=sys.stderr)


def check_path_encoding():
    current_path = str(Path(__file__).parent.parent.resolve())
    if not current_path.isascii():
        ui.clear()
        ui.box_output(
            [
                get_string("critical_error_path_encoding"),
                "-" * 75,
                get_string("current_path").format(current_path=current_path),
                "-" * 75,
                get_string("path_encoding_details_1"),
                get_string("path_encoding_details_2"),
                "",
                get_string("action_required"),
                get_string("action_required_details"),
                get_string("example_path"),
            ],
            err=True,
        )

        input(get_string("press_enter_to_continue"))
        raise RuntimeError(get_string("critical_error_path_encoding"))


# --- Task Execution ---


def run_task(
    command: str,
    dev: Any,
    registry: CommandRegistry,
    extra_kwargs: Optional[Dict[str, Any]] = None,
):
    ui.clear()

    cmd_info = registry.get(command)
    if not cmd_info:
        raise ToolError(get_string("unknown_command").format(command=command))

    title = cmd_info.title
    func = cmd_info.func
    base_kwargs = cmd_info.default_kwargs
    require_dev = cmd_info.require_dev

    try:
        if dev and hasattr(dev, "reset_task_state"):
            dev.reset_task_state()

        final_kwargs = base_kwargs.copy()

        if extra_kwargs:
            final_kwargs.update(extra_kwargs)

        if require_dev:
            final_kwargs["dev"] = dev

        result = func(**final_kwargs)

        if isinstance(result, str) and result:
            ui.echo(result)
        elif isinstance(result, tuple) and command == "read_anti_rollback":
            ui.echo(get_string("act_arb_complete").format(status=result[0]))
            ui.echo(get_string("act_curr_boot_idx").format(idx=result[1]))
            ui.echo(get_string("act_curr_vbmeta_idx").format(idx=result[2]))
        elif result:
            ui.echo(get_string("act_unhandled_success_result").format(res=result))

    except LTBoxError as e:
        ui.box_output([get_string("task_failed").format(title=title), str(e)], err=True)
    except subprocess.CalledProcessError as e:
        ui.box_output(_format_command_failure_messages(e), err=True)
    except (FileNotFoundError, RuntimeError, KeyError) as e:
        if not isinstance(e, SystemExit):
            ui.box_output([get_string("unexpected_error").format(e=e)], err=True)
    except SystemExit:
        ui.error(get_string("process_halted"))
    except KeyboardInterrupt:
        ui.error(get_string("process_cancelled"))
    finally:
        if dev and hasattr(dev, "adb"):
            dev.adb.force_kill_server()
        if dev and hasattr(dev, "fastboot"):
            dev.fastboot.force_kill_server()

        ui.echo("")
        input(get_string("press_enter_to_continue"))


def run_info_scan(paths, constants, avb_patch):
    print(get_string("scan_start"))

    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    log_filename = constants.BASE_DIR / f"image_info_{timestamp}.txt"

    files_to_scan = []
    for path_str in paths:
        p = Path(path_str)
        if p.is_dir():
            files_to_scan.extend(p.rglob("*.img"))
        elif p.is_file() and p.suffix.lower() == ".img":
            files_to_scan.append(p)

    if not files_to_scan:
        print(get_string("scan_no_files"), file=sys.stderr)
        return

    print(get_string("scan_found_files").format(count=len(files_to_scan)))

    with logging_context(log_filename) as logger:
        for f in files_to_scan:
            header = get_string("scan_log_header").format(path=f.resolve())
            logger.info(header)
            print(get_string("scan_scanning_file").format(filename=f.name))

            try:
                cmd = [
                    str(constants.PYTHON_EXE),
                    str(constants.AVBTOOL_PY),
                    "info_image",
                    "--image",
                    str(f),
                ]

                result = avb_patch.utils.run_command(cmd, capture=True, check=False)

                logger.info(result.stdout.strip())

                if result.stderr:
                    logger.info(
                        get_string("scan_log_errors").format(
                            errors=result.stderr.strip()
                        )
                    )

                logger.info("\n" + "=" * 78 + "\n")
            except Exception as e:
                error_msg = get_string("scan_failed").format(filename=f.name, e=e)
                print(error_msg, file=sys.stderr)
                logger.info(error_msg)

    print(get_string("scan_complete"))
    print(get_string("scan_saved_to").format(filename=log_filename.name))


# --- Menus ---


def advanced_menu(dev, registry: CommandRegistry, target_region: str):
    while True:
        menu_items = get_advanced_menu_data(target_region)
        action = select_menu_action(menu_items, "menu_adv_title")

        if action == "back":
            return
        elif action == "return":
            return
        elif action == "exit":
            sys.exit()
        elif action:
            extras = {}
            if action == "convert":
                extras["target_region"] = target_region
            run_task(action, dev, registry, extra_kwargs=extras)


def _root_action_menu(dev, registry: CommandRegistry, gki: bool, root_type: str):
    while True:
        menu_items = menu_data.get_root_menu_data(gki)
        action = select_menu_action(menu_items, "menu_root_title")

        if action == "back":
            return
        elif action == "return":
            return "main"
        elif action == "exit":
            sys.exit()
        elif action:
            extras = {}
            if not gki:
                extras["root_type"] = root_type
            run_task(action, dev, registry, extra_kwargs=extras)


def root_menu(dev, registry: CommandRegistry):
    while True:
        mode_menu = TerminalMenu(get_string("menu_root_type_title"))
        mode_menu.add_option("1", get_string("menu_root_type_magisk"))
        mode_menu.add_option("2", get_string("menu_root_type_ksu_next"))
        mode_menu.add_option("3", get_string("menu_root_type_sukisu"))
        mode_menu.add_separator()
        mode_menu.add_option("b", get_string("menu_back"))
        mode_menu.add_option("x", get_string("menu_main_exit"))

        choice = mode_menu.ask(
            get_string("prompt_select"), get_string("err_invalid_selection")
        )

        if choice == "1":
            result = _root_action_menu(dev, registry, gki=False, root_type="magisk")
        elif choice == "2":
            result = None
            while True:
                mode_items = TerminalMenu(get_string("menu_root_mode_title"))
                mode_items.add_option("1", get_string("menu_root_mode_1"))
                mode_items.add_option("2", get_string("menu_root_mode_2"))
                mode_items.add_separator()
                mode_items.add_option("b", get_string("menu_back"))
                mode_items.add_option("m", get_string("menu_root_m"))

                mode_choice = mode_items.ask(
                    get_string("prompt_select"), get_string("err_invalid_selection")
                )
                if mode_choice == "1":
                    result = _root_action_menu(
                        dev, registry, gki=False, root_type="ksu"
                    )
                    break
                if mode_choice == "2":
                    result = _root_action_menu(dev, registry, gki=True, root_type="ksu")
                    break
                if mode_choice == "b":
                    result = None
                    break
                if mode_choice == "m":
                    return
        elif choice == "3":
            result = _root_action_menu(dev, registry, gki=False, root_type="sukisu")
        elif choice == "b":
            return
        elif choice == "x":
            sys.exit()
        else:
            result = None

        if result == "main":
            return


def settings_menu(
    dev,
    registry: CommandRegistry,
    skip_adb: bool,
    skip_rollback: bool,
    target_region: str,
) -> Tuple[bool, bool, str]:
    while True:
        skip_adb_state = "ON" if skip_adb else "OFF"
        skip_rb_state = "ON" if skip_rollback else "OFF"

        menu_items = get_settings_menu_data(
            skip_adb_state, skip_rb_state, target_region
        )
        action = select_menu_action(menu_items, "menu_settings_title")

        if action == "back":
            return skip_adb, skip_rollback, target_region
        elif action == "return":
            return skip_adb, skip_rollback, target_region
        elif action == "toggle_region":
            target_region = "ROW" if target_region == "PRC" else "PRC"
            _save_settings({"target_region": target_region})
        elif action == "toggle_adb":
            skip_adb = not skip_adb
            dev.skip_adb = skip_adb
        elif action == "toggle_rollback":
            skip_rollback = not skip_rollback
        elif action == "change_lang":
            cmd_info = registry.get("change_language")
            if cmd_info:
                cmd_info.func()
        elif action == "check_update":
            ui.clear()
            ui.echo(get_string("act_update_checking"))

            current_version = _read_current_version()
            latest_version, latest_release, latest_prerelease = _get_latest_version(
                current_version
            )

            if latest_version:
                ui.echo(get_string("update_avail_title"))
                prompt_msg = get_string("update_avail_prompt").format(
                    curr=current_version, new=latest_version
                )
                choice = input(prompt_msg).strip().lower()
                if choice == "y":
                    ui.echo(get_string("update_open_web"))
                    webbrowser.open("https://github.com/miner7222/LTBox/releases")
                    sys.exit(0)
            else:
                if latest_release or latest_prerelease:
                    ui.echo(
                        get_string("act_update_not_found").format(
                            version=current_version
                        )
                    )
                else:
                    ui.echo(get_string("act_update_error").format(e="Unknown version"))

            ui.echo("")
            input(get_string("press_enter_to_continue"))


def prompt_for_language(force_prompt: bool = False) -> str:
    if not force_prompt:
        settings = _load_settings()
        saved_lang = settings.get("language")

        if saved_lang:
            try:
                available_languages = i18n.get_available_languages()
                avail_codes = [code for code, _ in available_languages]

                if saved_lang in avail_codes:
                    return saved_lang
            except Exception:
                pass

    i18n.load_lang("en")

    try:
        available_languages = i18n.get_available_languages()
    except RuntimeError as e:
        print(get_string("err_lang_generic").format(e=e), file=sys.stderr)
        input(get_string("press_enter_to_continue"))
        raise e

    menu = TerminalMenu(get_string("menu_lang_title"))
    lang_map = {}

    for i, (lang_code, lang_name) in enumerate(available_languages, 1):
        key = str(i)
        lang_map[key] = lang_code
        menu.add_option(key, lang_name)

    prompt = get_string("prompt_select").format(len=len(lang_map))
    error_msg = get_string("err_invalid_selection").format(len=len(lang_map))

    choice = menu.ask(prompt, error_msg)
    selected_lang = lang_map[choice]

    _save_settings({"language": selected_lang})

    return selected_lang


def main_loop(device_controller_class, registry: CommandRegistry):
    settings = _load_settings()

    skip_adb = False
    skip_rollback = False
    target_region = settings.get("target_region", "PRC")

    dev = device_controller_class(skip_adb=skip_adb)

    while True:
        menu_items = get_main_menu_data(target_region)
        action = select_menu_action(menu_items, "menu_main_title")

        if action == "exit":
            break
        elif action == "menu_settings":
            skip_adb, skip_rollback, target_region = settings_menu(
                dev, registry, skip_adb, skip_rollback, target_region
            )
        elif action == "menu_root":
            root_menu(dev, registry)
        elif action == "menu_advanced":
            advanced_menu(dev, registry, target_region)
        elif action:
            extras = {}
            if action in ["patch_all", "patch_all_wipe"]:
                extras["skip_rollback"] = skip_rollback
                extras["target_region"] = target_region
            run_task(action, dev, registry, extra_kwargs=extras)


# --- Singleton Check ---


def _acquire_single_instance_mutex() -> Optional[Any]:
    import ctypes

    if sys.platform != "win32":
        return "Non-Windows-Mutex"

    kernel32 = ctypes.windll.kernel32
    mutex_name = "Global\\LTBox_Singleton_Mutex"

    mutex = kernel32.CreateMutexW(None, False, mutex_name)

    if kernel32.GetLastError() == 183:
        return None

    return mutex


# --- Entry Point ---


def entry_point():
    try:
        _check_platform()
        setup_console()

        is_info_mode = len(sys.argv) > 1 and sys.argv[1].lower() == "info"

        if is_info_mode:
            lang_code = "en"
        else:
            lang_code = prompt_for_language()

        i18n.load_lang(lang_code)

        singleton_mutex = _acquire_single_instance_mutex()
        if not singleton_mutex:
            ui.clear()
            ui.error(get_string("err_already_running"))
            input()
            sys.exit(0)

        ui.clear()

        from . import utils

        current_version = _read_current_version()
        latest_version, _, _ = _get_latest_version(current_version)

        if latest_version:
            ui.echo(get_string("update_avail_title"))

            prompt_msg = get_string("update_avail_prompt").format(
                curr=current_version, new=latest_version
            )
            choice = input(prompt_msg).strip().lower()

            if choice == "y":
                ui.echo(get_string("update_open_web"))
                webbrowser.open("https://github.com/miner7222/LTBox/releases")
                sys.exit(0)

            ui.clear()

        try:
            downloader.install_base_tools(lang_code)
        except (subprocess.CalledProcessError, FileNotFoundError, ToolError) as e:
            ui.error(get_string("critical_err_base_tools").format(e=e))
            ui.error(get_string("err_run_install_manually"))
            input(get_string("press_enter_to_exit"))
            sys.exit(1)

        utils.check_dependencies()

        try:
            from . import actions, constants, device, utils, workflow
            from .patch import avb

            registry = CommandRegistry()

            @registry.register(
                "change_language", get_string("lang_changed"), require_dev=False
            )
            def change_language_task():
                new_lang = prompt_for_language(force_prompt=True)
                i18n.load_lang(new_lang)
                return get_string("lang_changed")

            command_specs = [
                (
                    "convert",
                    actions.convert_region_images,
                    get_string("task_title_convert_rom"),
                    True,
                    {},
                ),
                (
                    "root_device_gki",
                    actions.root_device,
                    get_string("task_title_root_gki"),
                    True,
                    {"gki": True},
                ),
                (
                    "patch_root_image_file_gki",
                    actions.patch_root_image_file,
                    get_string("task_title_root_file_gki"),
                    False,
                    {"gki": True},
                ),
                (
                    "patch_root_image_file_flash_gki",
                    actions.patch_root_image_file_and_flash,
                    get_string("task_title_root_file_gki"),
                    True,
                    {"gki": True},
                ),
                (
                    "root_device_lkm",
                    actions.root_device,
                    get_string("task_title_root_lkm"),
                    True,
                    {"gki": False},
                ),
                (
                    "patch_root_image_file_lkm",
                    actions.patch_root_image_file,
                    get_string("task_title_root_file_lkm"),
                    False,
                    {"gki": False},
                ),
                (
                    "patch_root_image_file_flash_lkm",
                    actions.patch_root_image_file_and_flash,
                    get_string("task_title_root_file_lkm"),
                    True,
                    {"gki": False},
                ),
                (
                    "unroot_device",
                    actions.unroot_device,
                    get_string("task_title_unroot"),
                    True,
                    {},
                ),
                (
                    "sign_and_flash_twrp",
                    actions.sign_and_flash_twrp,
                    get_string("task_title_rec_flash"),
                    True,
                    {},
                ),
                (
                    "disable_ota",
                    actions.disable_ota,
                    get_string("task_title_disable_ota"),
                    True,
                    {},
                ),
                (
                    "rescue_ota",
                    actions.rescue_after_ota,
                    get_string("task_title_rescue"),
                    True,
                    {},
                ),
                (
                    "edit_dp",
                    actions.edit_devinfo_persist,
                    get_string("task_title_patch_devinfo"),
                    False,
                    {},
                ),
                (
                    "dump_partitions",
                    actions.dump_partitions,
                    get_string("task_title_dump_devinfo"),
                    True,
                    {},
                ),
                (
                    "flash_partitions",
                    actions.flash_partitions,
                    get_string("task_title_write_devinfo"),
                    True,
                    {},
                ),
                (
                    "read_anti_rollback",
                    actions.read_anti_rollback_from_device,
                    get_string("task_title_read_arb"),
                    True,
                    {},
                ),
                (
                    "patch_anti_rollback",
                    actions.patch_anti_rollback_in_rom,
                    get_string("task_title_patch_arb"),
                    False,
                    {},
                ),
                (
                    "write_anti_rollback",
                    actions.write_anti_rollback,
                    get_string("task_title_write_arb"),
                    True,
                    {},
                ),
                (
                    "decrypt_xml",
                    actions.decrypt_x_files,
                    get_string("task_title_decrypt_xml"),
                    False,
                    {},
                ),
                (
                    "modify_xml",
                    actions.modify_xml,
                    get_string("task_title_modify_xml_nowipe"),
                    False,
                    {"wipe": 0},
                ),
                (
                    "modify_xml_wipe",
                    actions.modify_xml,
                    get_string("task_title_modify_xml_wipe"),
                    False,
                    {"wipe": 1},
                ),
                (
                    "flash_full_firmware",
                    actions.flash_full_firmware,
                    get_string("task_title_flash_full_firmware"),
                    True,
                    {},
                ),
                (
                    "patch_all",
                    workflow.patch_all,
                    get_string("task_title_install_nowipe"),
                    True,
                    {"wipe": 0},
                ),
                (
                    "patch_all_wipe",
                    workflow.patch_all,
                    get_string("task_title_install_wipe"),
                    True,
                    {"wipe": 1},
                ),
            ]

            for name, func, title, require_dev, extra_kwargs in command_specs:
                registry.add(name, func, title, require_dev=require_dev, **extra_kwargs)

            device_controller_class = device.DeviceController
            constants_module = constants
            avb_patch_module = avb

        except ImportError as e:
            ui.error(get_string("err_import_ltbox"))
            ui.error(get_string("err_details").format(e=e))
            ui.error(get_string("err_ensure_ltbox_present"))
            input(get_string("press_enter_to_exit"))
            sys.exit(1)

        check_path_encoding()

        if is_info_mode:
            if len(sys.argv) > 2:
                run_info_scan(sys.argv[2:], constants_module, avb_patch_module)
            else:
                ui.error(get_string("info_no_files_dragged"))
                ui.error(get_string("info_drag_files_prompt"))

            input(get_string("press_enter_to_exit"))
        else:
            main_loop(device_controller_class, registry)

    except (LTBoxError, RuntimeError) as e:
        ui.error(get_string("err_fatal_abort"))
        ui.error(get_string("err_details").format(e=e))
        input(get_string("press_enter_to_exit"))
        sys.exit(1)
    except KeyboardInterrupt:
        ui.error(get_string("err_fatal_user_cancel"))
        sys.exit(0)


if __name__ == "__main__":
    entry_point()
