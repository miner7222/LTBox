import os
import platform
import subprocess
import sys
import json
from pathlib import Path
from datetime import datetime
from typing import Tuple, Dict, Callable, Any

from . import downloader, i18n
from .i18n import get_string
from .logger import logging_context
from .utils import ui

APP_DIR = Path(__file__).parent.resolve()
BASE_DIR = APP_DIR.parent
PYTHON_EXE = BASE_DIR / "python3" / "python.exe"
SETTINGS_FILE = APP_DIR / "settings.json"

try:
    from .errors import ToolError
except ImportError:
    print(get_string("err_import_critical"), file=sys.stderr)
    print(get_string("err_ensure_errors"), file=sys.stderr)
    input(get_string("press_enter_to_exit"))
    sys.exit(1)

def _load_settings() -> Dict[str, Any]:
    if SETTINGS_FILE.exists():
        try:
            with open(SETTINGS_FILE, 'r', encoding='utf-8') as f:
                return json.load(f)
        except Exception:
            return {}
    return {}

def _save_settings(data: Dict[str, Any]) -> None:
    try:
        current_settings = _load_settings()
        current_settings.update(data)
        with open(SETTINGS_FILE, 'w', encoding='utf-8') as f:
            json.dump(current_settings, f, indent=2)
    except Exception as e:
        print(f"Warning: Failed to save settings: {e}", file=sys.stderr)

def _check_platform():
    if platform.system() != "Windows":
        print(get_string("err_fatal_windows"), file=sys.stderr)
        print(get_string("err_current_platform").format(platform=platform.system()), file=sys.stderr)
        print(get_string("err_aborting"), file=sys.stderr)
        input(get_string("press_enter_to_exit"))
        sys.exit(1)
    
    if platform.machine() != "AMD64":
        print(get_string("err_fatal_amd64"), file=sys.stderr)
        print(get_string("err_current_arch").format(arch=platform.machine()), file=sys.stderr)
        print(get_string("err_arch_unsupported"), file=sys.stderr)
        print(get_string("err_aborting"), file=sys.stderr)
        input(get_string("press_enter_to_exit"))
        sys.exit(1)

def setup_console():
    try:
        import ctypes
        ctypes.windll.kernel32.SetConsoleTitleW(u"LTBox")

        sys.stdout.write("\x1b[8;40;80t")
        sys.stdout.flush()

        os.system("mode con: cols=80 lines=40")
        
    except Exception as e:
        print(get_string("warn_set_console_title").format(e=e), file=sys.stderr)

def check_path_encoding():
    current_path = str(Path(__file__).parent.parent.resolve())
    if not current_path.isascii():
        ui.clear()
        ui.box_output([
            get_string("critical_error_path_encoding"),
            "  " + "-" * 75,
            get_string("current_path").format(current_path=current_path),
            "  " + "-" * 75,
            get_string("path_encoding_details_1"),
            get_string("path_encoding_details_2"),
            "",
            get_string("action_required"),
            get_string("action_required_details"),
            get_string("example_path")
        ], err=True)
        
        input(get_string("press_enter_to_continue"))
        raise RuntimeError(get_string("critical_error_path_encoding"))

def run_task(command, title, dev, command_map, extra_kwargs=None):
    ui.clear()
    
    ui.echo("  " + "=" * 78)
    ui.echo(get_string("starting_task").format(title=title))
    ui.echo("  " + "=" * 78 + "\n")

    try:
        func_tuple = command_map.get(command)
        if not func_tuple:
            raise ToolError(get_string("unknown_command").format(command=command))
        
        func, base_kwargs = func_tuple
        final_kwargs = base_kwargs.copy()
        
        if extra_kwargs:
            final_kwargs.update(extra_kwargs)
        
        no_dev_needed = {
            "patch_root_image_file_gki", "patch_root_image_file_lkm", 
            "edit_dp", 
            "patch_anti_rollback", "clean", "modify_xml", "modify_xml_wipe",
            "decrypt_xml", "change_language"
        }
        
        if command not in no_dev_needed:
            final_kwargs["dev"] = dev

        result = func(**final_kwargs)

        ui.echo("\n  " + "=" * 78)
        ui.echo(get_string("act_success"))
        ui.echo("  " + "=" * 78)

        if isinstance(result, str) and result:
            ui.echo(result)
        elif isinstance(result, tuple) and command == "read_anti_rollback":
                ui.echo(get_string("act_arb_complete").format(status=result[0]))
                ui.echo(get_string("act_curr_boot_idx").format(idx=result[1]))
                ui.echo(get_string("act_curr_vbmeta_idx").format(idx=result[2]))
        elif command == "clean":
            pass
        elif result:
            ui.echo(get_string("act_unhandled_success_result").format(res=result))

    except ToolError as e:
        ui.box_output([get_string("task_failed").format(title=title), str(e)], err=True)
    except subprocess.CalledProcessError as e:
        msgs = [get_string("err_cmd_failed").format(cmd=" ".join(e.cmd) if isinstance(e.cmd, list) else e.cmd)]
        if e.stdout:
            msgs.append(f"{get_string('err_cmd_stdout_header')}\n{e.stdout}")
        if e.stderr:
            msgs.append(f"{get_string('err_cmd_stderr_header')}\n{e.stderr}")
        ui.box_output(msgs, err=True)
    except (FileNotFoundError, RuntimeError, KeyError) as e:
        if not isinstance(e, SystemExit):
            ui.box_output([get_string("unexpected_error").format(e=e)], err=True)
    except SystemExit:
        ui.error(get_string("process_halted"))
    except KeyboardInterrupt:
        ui.error(get_string("process_cancelled"))
    finally:
        ui.echo("")

        ui.echo("  " + "=" * 78)
        ui.echo(get_string("task_completed").format(title=title))
        ui.echo("  " + "=" * 78 + "\n")
        
        if command == "clean":
            input(get_string("press_enter_to_exit"))
        else:
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
        elif p.is_file() and p.suffix.lower() == '.img':
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
                    str(f)
                ]
                
                result = avb_patch.utils.run_command(cmd, capture=True, check=False)
                
                logger.info(result.stdout.strip())
                
                if result.stderr:
                     logger.info(get_string("scan_log_errors").format(errors=result.stderr.strip()))

                logger.info("\n" + "="*78 + "\n")
            except Exception as e:
                error_msg = get_string("scan_failed").format(filename=f.name, e=e)
                print(error_msg, file=sys.stderr)
                logger.info(error_msg)
    
    print(get_string("scan_complete"))
    print(get_string("scan_saved_to").format(filename=log_filename.name))

def print_main_menu(skip_adb, skip_rollback):
    skip_adb_state = "ON" if skip_adb else "OFF"
    skip_rb_state = "ON" if skip_rollback else "OFF"
    os.system('cls')
    print("\n  " + "=" * 78)
    print(get_string("menu_main_title"))
    print("  " + "=" * 78 + "\n")

    print(f"  {get_string('menu_main_sub_install')}")
    print(get_string("menu_main_install_wipe"))
    print(get_string("menu_main_install_keep"))
    print("")

    print(f"  {get_string('menu_main_sub_manage')}")
    print(get_string("menu_main_disable_ota"))
    print(get_string("menu_main_rescue"))
    print(get_string("menu_main_root"))
    print(get_string("menu_main_unroot"))
    print("")

    print(f"  {get_string('menu_main_sub_settings')}")
    print(get_string("menu_main_skip_adb").format(skip_adb_state=skip_adb_state))
    print(get_string("menu_main_skip_rb").format(skip_rb_state=skip_rb_state))
    print(get_string("menu_main_language"))
    print("")

    print(f"  {get_string('menu_main_sub_nav')}")
    print(get_string("menu_main_adv"))
    print(get_string("menu_main_exit"))
    print("\n  " + "=" * 78 + "\n")

def print_advanced_menu():
    os.system('cls')
    print("\n  " + "=" * 78)
    print(get_string("menu_adv_title"))
    print("  " + "=" * 78 + "\n")

    print(f"  {get_string('menu_adv_sub_region_dump')}")
    print(get_string("menu_adv_1"))
    print(get_string("menu_adv_2"))
    print("")

    print(f"  {get_string('menu_adv_sub_patch_region')}")
    print(get_string("menu_adv_3"))
    print(get_string("menu_adv_4"))
    print("")

    print(f"  {get_string('menu_adv_sub_arb')}")
    print(get_string("menu_adv_5"))
    print(get_string("menu_adv_6"))
    print(get_string("menu_adv_7"))
    print("")

    print(f"  {get_string('menu_adv_sub_xml_flash')}")
    print(get_string("menu_adv_8"))
    print(get_string("menu_adv_9"))
    print(get_string("menu_adv_10"))
    print(get_string("menu_adv_11"))
    print("")

    print(f"  {get_string('menu_adv_sub_maint')}")
    print(get_string("menu_adv_12"))
    print("")

    print(f"  {get_string('menu_adv_sub_nav')}")
    print(get_string("menu_adv_m"))
    print(get_string("menu_main_exit"))
    print("\n  " + "=" * 78 + "\n")

def advanced_menu(dev, command_map):
    actions_map = {
        "1": ("convert", get_string("task_title_convert_rom")),
        "2": ("dump_partitions", get_string("task_title_dump_devinfo")),
        "3": ("edit_dp", get_string("task_title_patch_devinfo")),
        "4": ("flash_partitions", get_string("task_title_write_devinfo")),
        "5": ("read_anti_rollback", get_string("task_title_read_arb")),
        "6": ("patch_anti_rollback", get_string("task_title_patch_arb")),
        "7": ("write_anti_rollback", get_string("task_title_write_arb")),
        "8": ("decrypt_xml", get_string("task_title_decrypt_xml")),
        "9": ("modify_xml_wipe", get_string("task_title_modify_xml_wipe")),
        "10": ("modify_xml", get_string("task_title_modify_xml_nowipe")),
        "11": ("flash_full_firmware", get_string("task_title_flash_full_firmware")),
        "12": ("clean", get_string("task_title_clean"))
    }

    while True:
        print_advanced_menu()
        choice = input(get_string("menu_adv_prompt")).strip().lower()

        if choice in actions_map:
            cmd, title = actions_map[choice]
            run_task(cmd, title, dev, command_map)
            if choice == "12":
                sys.exit()
        elif choice == "m":
            return
        elif choice == "x":
            sys.exit()
        else:
            print(get_string("menu_adv_invalid"))
            input(get_string("press_enter_to_continue"))

def print_root_mode_selection_menu():
    os.system('cls')
    print("\n  " + "=" * 78)
    print(get_string("menu_root_mode_title"))
    print("  " + "=" * 78 + "\n")
    print(get_string("menu_root_mode_1"))
    print(get_string("menu_root_mode_2"))
    print("\n" + get_string("menu_root_m"))
    print(get_string("menu_main_exit"))
    print("\n  " + "=" * 78 + "\n")

def root_mode_selection_menu(dev, command_map):
    while True:
        print_root_mode_selection_menu()
        choice = input(get_string("menu_root_mode_prompt")).strip().lower()

        if choice == "1":
            root_menu(dev, command_map, gki=False)
        elif choice == "2":
            root_menu(dev, command_map, gki=True)
        elif choice == "m":
            return
        elif choice == "x":
            sys.exit()
        else:
            print(get_string("menu_root_mode_invalid"))
            input(get_string("press_enter_to_continue"))

def print_root_menu(gki: bool):
    os.system('cls')
    print("\n  " + "=" * 78)
    print(get_string("menu_root_title"))
    print("  " + "=" * 78 + "\n")
    if gki:
        print(get_string("menu_root_1_gki"))
        print(get_string("menu_root_2_gki"))
    else:
        print(get_string("menu_root_1_lkm"))
        print(get_string("menu_root_2_lkm"))
    print("\n" + get_string("menu_root_m"))
    print(get_string("menu_main_exit"))
    print("\n  " + "=" * 78 + "\n")

def root_menu(dev, command_map, gki: bool):
    if gki:
        actions_map = {
            "1": ("root_device_gki", get_string("task_title_root_gki")),
            "2": ("patch_root_image_file_gki", get_string("task_title_root_file_gki")),
        }
    else:
        actions_map = {
            "1": ("root_device_lkm", get_string("task_title_root_lkm")),
            "2": ("patch_root_image_file_lkm", get_string("task_title_root_file_lkm")),
        }

    while True:
        print_root_menu(gki)
        choice = input(get_string("menu_root_prompt")).strip().lower()

        if choice in actions_map:
            cmd, title = actions_map[choice]
            run_task(cmd, title, dev, command_map)
        elif choice == "m":
            return
        elif choice == "x":
            sys.exit()
        else:
            print(get_string("menu_root_invalid"))
            input(get_string("press_enter_to_continue"))

def change_language_task():
    new_lang = prompt_for_language(force_prompt=True)
    i18n.load_lang(new_lang)
    return get_string("lang_changed")

def main_loop(device_controller_class, command_map):
    skip_adb = False
    skip_rollback = False
    dev = device_controller_class(skip_adb=skip_adb)
    
    actions_map = {
        "1": ("patch_all_wipe", get_string("task_title_install_wipe")),
        "2": ("patch_all", get_string("task_title_install_nowipe")),
        "3": ("disable_ota", get_string("task_title_disable_ota")),
        "4": ("rescue_ota", get_string("task_title_rescue")),
        "6": ("unroot_device", get_string("task_title_unroot"))
    }

    while True:
        print_main_menu(skip_adb, skip_rollback)
        choice = input(get_string("menu_main_prompt")).strip().lower()

        if choice in actions_map:
            cmd, title = actions_map[choice]
            extras = {}
            if cmd in ["patch_all", "patch_all_wipe"]:
                extras["skip_rollback"] = skip_rollback
            run_task(cmd, title, dev, command_map, extra_kwargs=extras)
        elif choice == "5":
            root_mode_selection_menu(dev, command_map)
        elif choice == "7":
            skip_adb = not skip_adb
            dev.skip_adb = skip_adb
        elif choice == "8":
            skip_rollback = not skip_rollback
        elif choice == "9":
            change_language_task()
        elif choice == "a":
            advanced_menu(dev, command_map)
        elif choice == "x":
            break
        else:
            print(get_string("menu_main_invalid"))
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
        if "Language directory not found" in str(e):
            print(get_string("err_lang_dir_not_found"), file=sys.stderr)
            print(get_string("err_lang_dir_expected").format(path=i18n.LANG_DIR), file=sys.stderr)
        elif "No language files" in str(e):
            print(get_string("err_no_lang_files"), file=sys.stderr)
            print(get_string("err_no_lang_files_path").format(path=i18n.LANG_DIR), file=sys.stderr)
        else:
            print(get_string("err_lang_generic").format(e=e), file=sys.stderr)
        
        input(get_string("press_enter_to_continue"))
        raise e

    menu_options = []
    lang_map = {}
    
    for i, (lang_code, lang_name) in enumerate(available_languages, 1):
        lang_map[str(i)] = lang_code
        menu_options.append(f"     {i}. {lang_name}")

    os.system('cls')
    print("\n  " + "=" * 78)
    print(get_string("menu_lang_title"))
    print("  " + "=" * 78 + "\n")
    print("\n".join(menu_options))
    print("\n  " + "=" * 78 + "\n")

    choice = ""
    while choice not in lang_map:
        prompt = get_string("menu_lang_prompt").format(len=len(lang_map))
        choice = input(prompt).strip()
        if choice not in lang_map:
            print(get_string("menu_lang_invalid").format(len=len(lang_map)))
    
    selected_lang = lang_map[choice]

    _save_settings({"language": selected_lang})
    
    return selected_lang

def entry_point():
    try:
        _check_platform()
        setup_console()
        
        is_info_mode = len(sys.argv) > 1 and sys.argv[1].lower() == 'info'
        
        if is_info_mode:
            lang_code = "en"
        else:
            lang_code = prompt_for_language()
            
        i18n.load_lang(lang_code)
        
        ui.clear()

        try:
            downloader.install_base_tools(lang_code)
        except (subprocess.CalledProcessError, FileNotFoundError, ToolError) as e:
            ui.error(get_string("critical_err_base_tools").format(e=e))
            ui.error(get_string("err_run_install_manually"))
            input(get_string("press_enter_to_exit"))
            sys.exit(1)

        try:
            from . import utils as u, actions as a, workflow as w, device as d
            from . import constants as c
            from .patch import avb as avb

            COMMAND_MAP: Dict[str, Tuple[Callable[..., Any], Dict[str, Any]]] = {
                "convert": (a.convert_region_images, {}),
                "root_device_gki": (a.root_device, {"gki": True}),
                "patch_root_image_file_gki": (a.patch_root_image_file, {"gki": True}),
                "root_device_lkm": (a.root_device, {"gki": False}),
                "patch_root_image_file_lkm": (a.patch_root_image_file, {"gki": False}),
                "unroot_device": (a.unroot_device, {}),
                "disable_ota": (a.disable_ota, {}),
                "rescue_ota": (a.rescue_after_ota, {}),
                "edit_dp": (a.edit_devinfo_persist, {}),
                "dump_partitions": (a.dump_partitions, {}),
                "flash_partitions": (a.flash_partitions, {}),
                "read_anti_rollback": (a.read_anti_rollback_from_device, {}),
                "patch_anti_rollback": (a.patch_anti_rollback_in_rom, {}),
                "write_anti_rollback": (a.write_anti_rollback, {}),
                "clean": (u.clean_workspace, {}),
                "decrypt_xml": (a.decrypt_x_files, {}),
                "modify_xml": (a.modify_xml, {"wipe": 0}),
                "modify_xml_wipe": (a.modify_xml, {"wipe": 1}),
                "flash_full_firmware": (a.flash_full_firmware, {}),
                "patch_all": (w.patch_all, {"wipe": 0}),
                "patch_all_wipe": (w.patch_all, {"wipe": 1}),
                "change_language": (change_language_task, {}),
            }
            
            device_controller_class = d.DeviceController
            constants_module = c
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
            main_loop(device_controller_class, COMMAND_MAP)

    except (RuntimeError, ToolError) as e:
        ui.error(get_string("err_fatal_abort"))
        ui.error(get_string("err_fatal_details").format(e=e))
        input(get_string("press_enter_to_exit"))
        sys.exit(1)
    except KeyboardInterrupt:
        ui.error(get_string("err_fatal_user_cancel"))
        sys.exit(0)

if __name__ == "__main__":
    entry_point()