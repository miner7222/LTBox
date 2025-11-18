import argparse
import os
import platform
import subprocess
import sys
import json
from pathlib import Path
from datetime import datetime
from typing import List, Tuple

from . import downloader, i18n
from .logger import logging_context

APP_DIR = Path(__file__).parent.resolve()
BASE_DIR = APP_DIR.parent
PYTHON_EXE = BASE_DIR / "python3" / "python.exe"
DOWNLOADER_PY = APP_DIR / "downloader.py"

try:
    from .errors import ToolError
except ImportError:
    print(f"[!] Critical Import Error: Failed to import 'ltbox.errors'.", file=sys.stderr)
    print(f"[!] Please ensure 'ltbox/errors.py' file exists.", file=sys.stderr)
    if platform.system() == "Windows":
        os.system("pause")
    sys.exit(1)

def _check_platform():
    if platform.system() != "Windows":
        print("[!] Fatal Error: This tool is designed to run only on Windows.", file=sys.stderr)
        print(f"    Current platform detected: {platform.system()}", file=sys.stderr)
        print("[!] Aborting.", file=sys.stderr)
        os.system("pause")
        sys.exit(1)
    
    if platform.machine() != "AMD64":
        print("[!] Fatal Error: This tool requires a 64-bit (AMD64) Windows environment.", file=sys.stderr)
        print(f"    Current architecture detected: {platform.machine()}", file=sys.stderr)
        print("[!] 32-bit (I386) or ARM64 builds are not supported.", file=sys.stderr)
        print("[!] Aborting.", file=sys.stderr)
        os.system("pause")
        sys.exit(1)

def setup_console():
    system = platform.system()
    if system == "Windows":
        try:
            import ctypes
            ctypes.windll.kernel32.SetConsoleTitleW(get_string("app_title"))
        except Exception as e:
            print(get_string("warn_set_console_title").format(e=e), file=sys.stderr)

from .i18n import get_string

def check_path_encoding():
    current_path = str(Path(__file__).parent.parent.resolve())
    if not current_path.isascii():
        os.system('cls' if os.name == 'nt' else 'clear')
        print("\n" + "!" * 65)
        print(get_string("critical_error_path_encoding"))
        print("  " + "-" * 60)
        print(get_string("current_path").format(current_path=current_path))
        print("  " + "-" * 60)
        print(get_string("path_encoding_details_1"))
        print(get_string("path_encoding_details_2"))
        print("\n" + get_string("action_required"))
        print(get_string("action_required_details"))
        print(get_string("example_path"))
        print("!" * 65 + "\n")
        
        if platform.system() == "Windows":
            os.system("pause")
        else:
            input(get_string("press_enter_to_exit"))
        raise RuntimeError(get_string("critical_error_path_encoding"))

def run_task(command, title, dev, command_map):
    os.system('cls' if os.name == 'nt' else 'clear')
    
    print("  " + "=" * 58)
    print(get_string("starting_task").format(title=title))
    print("  " + "=" * 58, "\n")

    log_file = None
    if command in ["patch_all", "patch_all_wipe"]:
        timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
        log_file = f"log_{timestamp}.txt"
        print(get_string("logging_enabled").format(log_file=log_file))
        print(get_string("logging_command").format(command=command))

    try:
        with logging_context(log_file):
            func_tuple = command_map.get(command)
            if not func_tuple:
                raise ToolError(get_string("unknown_command").format(command=command))
            
            func, base_kwargs = func_tuple
            final_kwargs = base_kwargs.copy()
            
            no_dev_needed = {
                "root_boot_only_gki", "root_boot_only_lkm", 
                "edit_dp", 
                "patch_anti_rollback", "clean", "modify_xml", "modify_xml_wipe",
                "decrypt_xml"
            }
            
            if command not in no_dev_needed:
                final_kwargs["dev"] = dev
            
            result = func(**final_kwargs)

            print("\n" + "=" * 61)
            print(get_string("act_success"))
            print("=" * 61)

            if isinstance(result, str) and result:
                print(result)
            elif isinstance(result, tuple) and command == "read_anti_rollback":
                 print(get_string("act_arb_complete").format(status=result[0]))
                 print(get_string("act_curr_boot_idx").format(idx=result[1]))
                 print(get_string("act_curr_vbmeta_idx").format(idx=result[2]))
            elif command == "clean":
                pass
            elif result:
                print(get_string("act_unhandled_success_result").format(res=result))


    except ToolError as e:
        print("\n" + "!" * 61, file=sys.stderr)
        print(get_string("task_failed").format(title=title), file=sys.stderr)
        print(str(e), file=sys.stderr)
        print("!" * 61, file=sys.stderr)
    except subprocess.CalledProcessError as e:
        print("\n" + "!" * 61, file=sys.stderr)
        cmd_str = " ".join(e.cmd) if isinstance(e.cmd, list) else e.cmd
        print(get_string("err_cmd_failed").format(cmd=cmd_str), file=sys.stderr)
        if e.stdout:
            print(f"{get_string('err_cmd_stdout_header')}\n{e.stdout}", file=sys.stderr)
        if e.stderr:
            print(f"{get_string('err_cmd_stderr_header')}\n{e.stderr}", file=sys.stderr)
        print("!" * 61, file=sys.stderr)
    except (FileNotFoundError, RuntimeError, KeyError) as e:
        if not isinstance(e, SystemExit):
            print("\n" + "!" * 61, file=sys.stderr)
            print(get_string("unexpected_error").format(e=e), file=sys.stderr)
            print("!" * 61, file=sys.stderr)
    except SystemExit:
        print(get_string("process_halted"), file=sys.stderr)
    except KeyboardInterrupt:
        print(get_string("process_cancelled"), file=sys.stderr)
    finally:
        print()
        if log_file:
            print(get_string("logging_finished").format(log_file=log_file))

        print("  " + "=" * 58)
        print(get_string("task_completed").format(title=title))
        print("  " + "=" * 58, "\n")
        
        if command == "clean":
            print(get_string("press_any_key_to_exit"))
        else:
            print(get_string("press_any_key_to_return"))

        if platform.system() == "Windows":
            os.system(f"pause > nul & echo {get_string('press_any_key')}")
        else:
            input()

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

                logger.info("\n" + "="*70 + "\n")
            except Exception as e:
                error_msg = get_string("scan_failed").format(filename=f.name, e=e)
                print(error_msg, file=sys.stderr)
                logger.info(error_msg)
    
    print(get_string("scan_complete"))
    print(get_string("scan_saved_to").format(filename=log_filename.name))

def print_main_menu(skip_adb):
    skip_adb_state = "ON" if skip_adb else "OFF"
    os.system('cls' if os.name == 'nt' else 'clear')
    print("\n  " + "=" * 58)
    print(get_string("menu_main_title"))
    print("  " + "=" * 58 + "\n")
    print(get_string("menu_main_1"))
    print(get_string("menu_main_2"))
    print(get_string("menu_main_3"))
    print(get_string("menu_main_4"))
    print(get_string("menu_main_5"))
    print(get_string("menu_main_6").format(skip_adb_state=skip_adb_state))
    print("\n" + get_string("menu_main_a"))
    print(get_string("menu_main_x"))
    print("\n  " + "=" * 58 + "\n")

def print_advanced_menu():
    os.system('cls' if os.name == 'nt' else 'clear')
    print("\n  " + "=" * 58)
    print(get_string("menu_adv_title"))
    print("  " + "=" * 58 + "\n")
    print(get_string("menu_adv_1"))
    print(get_string("menu_adv_2"))
    print(get_string("menu_adv_3"))
    print(get_string("menu_adv_4"))
    print(get_string("menu_adv_5"))
    print(get_string("menu_adv_6"))
    print(get_string("menu_adv_7"))
    print(get_string("menu_adv_8"))
    print(get_string("menu_adv_9"))
    print(get_string("menu_adv_10"))
    print(get_string("menu_adv_11"))
    print(get_string("menu_adv_12"))
    print("\n" + get_string("menu_adv_m"))
    print("\n  " + "=" * 58 + "\n")

def advanced_menu(dev, command_map):
    actions_map = {
        "1": ("convert", get_string("task_title_convert_rom")),
        "2": ("read_edl", get_string("task_title_dump_devinfo")),
        "3": ("edit_dp", get_string("task_title_patch_devinfo")),
        "4": ("write_edl", get_string("task_title_write_devinfo")),
        "5": ("read_anti_rollback", get_string("task_title_read_arb")),
        "6": ("patch_anti_rollback", get_string("task_title_patch_arb")),
        "7": ("write_anti_rollback", get_string("task_title_write_arb")),
        "8": ("decrypt_xml", get_string("task_title_decrypt_xml")),
        "9": ("modify_xml_wipe", get_string("task_title_modify_xml_wipe")),
        "10": ("modify_xml", get_string("task_title_modify_xml_nowipe")),
        "11": ("flash_edl", get_string("task_title_flash_edl")),
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
        else:
            print(get_string("menu_adv_invalid"))
            if platform.system() == "Windows":
                os.system(f"pause > nul & echo {get_string('press_any_key')}...")
            else:
                input(get_string("press_enter_to_continue"))

def print_root_mode_selection_menu():
    os.system('cls' if os.name == 'nt' else 'clear')
    print("\n  " + "=" * 58)
    print(get_string("menu_root_mode_title"))
    print("  " + "=" * 58 + "\n")
    print(get_string("menu_root_mode_1"))
    print(get_string("menu_root_mode_2"))
    print("\n" + get_string("menu_root_m"))
    print("\n  " + "=" * 58 + "\n")

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
        else:
            print(get_string("menu_root_mode_invalid"))
            if platform.system() == "Windows":
                os.system(f"pause > nul & echo {get_string('press_any_key')}...")
            else:
                input(get_string("press_enter_to_continue"))

def print_root_menu(gki: bool):
    os.system('cls' if os.name == 'nt' else 'clear')
    print("\n  " + "=" * 58)
    print(get_string("menu_root_title"))
    print("  " + "=" * 58 + "\n")
    if gki:
        print(get_string("menu_root_1_gki"))
        print(get_string("menu_root_2_gki"))
    else:
        print(get_string("menu_root_1_lkm"))
        print(get_string("menu_root_2_lkm"))
    print("\n" + get_string("menu_root_m"))
    print("\n  " + "=" * 58 + "\n")

def root_menu(dev, command_map, gki: bool):
    if gki:
        actions_map = {
            "1": ("root_boot_only_gki", get_string("task_title_root_file_gki")),
            "2": ("root_device_gki", get_string("task_title_root_gki")),
        }
    else:
        actions_map = {
            "1": ("root_boot_only_lkm", get_string("task_title_root_file_lkm")),
            "2": ("root_device_lkm", get_string("task_title_root_lkm")),
        }

    while True:
        print_root_menu(gki)
        choice = input(get_string("menu_root_prompt")).strip().lower()

        if choice in actions_map:
            cmd, title = actions_map[choice]
            run_task(cmd, title, dev, command_map)
        elif choice == "m":
            return
        else:
            print(get_string("menu_root_invalid"))
            if platform.system() == "Windows":
                os.system(f"pause > nul & echo {get_string('press_any_key')}...")
            else:
                input(get_string("press_enter_to_continue"))

def main_loop(device_controller_class, command_map):
    skip_adb = False
    dev = device_controller_class(skip_adb=skip_adb)
    
    actions_map = {
        "1": ("patch_all_wipe", get_string("task_title_install_wipe")),
        "2": ("patch_all", get_string("task_title_install_nowipe")),
        "3": ("disable_ota", get_string("task_title_disable_ota")),
        "5": ("unroot_device", get_string("task_title_unroot")),
    }

    while True:
        print_main_menu(skip_adb)
        choice = input(get_string("menu_main_prompt")).strip().lower()

        if choice in actions_map:
            cmd, title = actions_map[choice]
            run_task(cmd, title, dev, command_map)
        elif choice == "4":
            root_mode_selection_menu(dev, command_map)
        elif choice == "6":
            skip_adb = not skip_adb
            dev.skip_adb = skip_adb
        elif choice == "a":
            advanced_menu(dev, command_map)
        elif choice == "x":
            break
        else:
            print(get_string("menu_main_invalid"))
            if platform.system() == "Windows":
                os.system(f"pause > nul & echo {get_string('press_any_key')}...")
            else:
                input(get_string("press_enter_to_continue"))

def prompt_for_language() -> str:
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
        
        if platform.system() == "Windows":
            os.system("pause")
        raise e

    menu_options = []
    lang_map = {}
    
    for i, (lang_code, lang_name) in enumerate(available_languages, 1):
        lang_map[str(i)] = lang_code
        menu_options.append(f"     {i}. {lang_name}")

    os.system('cls' if os.name == 'nt' else 'clear')
    print("\n  " + "=" * 58)
    print(get_string("menu_lang_title"))
    print("  " + "=" * 58 + "\n")
    print("\n".join(menu_options))
    print("\n  " + "=" * 58 + "\n")

    choice = ""
    while choice not in lang_map:
        prompt = get_string("menu_lang_prompt").format(len=len(lang_map))
        choice = input(prompt).strip()
        if choice not in lang_map:
            print(get_string("menu_lang_invalid").format(len=len(lang_map)))
    
    return lang_map[choice]

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
        
        os.system('cls' if os.name == 'nt' else 'clear')

        try:
            downloader.install_base_tools(lang_code)
        except (subprocess.CalledProcessError, FileNotFoundError, ToolError) as e:
            print(get_string("critical_err_base_tools").format(e=e), file=sys.stderr)
            print(get_string("err_run_install_manually"), file=sys.stderr)
            if platform.system() == "Windows":
                os.system("pause")
            sys.exit(1)

        try:
            from . import utils as u, actions as a, workflow as w, device as d
            from . import constants as c
            from .patch import avb as avb
            
            COMMAND_MAP = {
                "convert": (a.convert_images, {}),
                "root_device_gki": (a.root_device, {"gki": True}),
                "root_boot_only_gki": (a.root_boot_only, {"gki": True}),
                "root_device_lkm": (a.root_device, {"gki": False}),
                "root_boot_only_lkm": (a.root_boot_only, {"gki": False}),
                "unroot_device": (a.unroot_device, {}),
                "disable_ota": (a.disable_ota, {}),
                "edit_dp": (a.edit_devinfo_persist, {}),
                "read_edl": (a.read_edl, {}),
                "write_edl": (a.write_edl, {}),
                "read_anti_rollback": (a.read_anti_rollback_from_device, {}),
                "patch_anti_rollback": (a.patch_anti_rollback_in_rom, {}),
                "write_anti_rollback": (a.write_anti_rollback, {}),
                "clean": (u.clean_workspace, {}),
                "decrypt_xml": (a.decrypt_x_files, {}),
                "modify_xml": (a.modify_xml, {"wipe": 0}),
                "modify_xml_wipe": (a.modify_xml, {"wipe": 1}),
                "flash_edl": (a.flash_edl, {}),
                "patch_all": (w.patch_all, {"wipe": 0}),
                "patch_all_wipe": (w.patch_all, {"wipe": 1}),
            }
            
            device_controller_class = d.DeviceController
            constants_module = c
            avb_patch_module = avb

        except ImportError as e:
            print(get_string("err_import_ltbox"), file=sys.stderr)
            print(get_string("err_details").format(e=e), file=sys.stderr)
            print(get_string("err_ensure_ltbox_present"), file=sys.stderr)
            if platform.system() == "Windows":
                os.system("pause")
            sys.exit(1)

        check_path_encoding()
        
        if is_info_mode:
            if len(sys.argv) > 2:
                run_info_scan(sys.argv[2:], constants_module, avb_patch_module)
            else:
                print(get_string("info_no_files_dragged"), file=sys.stderr)
                print(get_string("info_drag_files_prompt"), file=sys.stderr)
            
            if platform.system() == "Windows":
                os.system("pause")
        else:
            main_loop(device_controller_class, COMMAND_MAP)

    except (RuntimeError, ToolError) as e:
        print(get_string("err_fatal_abort"), file=sys.stderr)
        print(get_string("err_fatal_details").format(e=e), file=sys.stderr)
        if platform.system() == "Windows":
            os.system("pause")
        sys.exit(1)
    except KeyboardInterrupt:
        print(get_string("err_fatal_user_cancel"), file=sys.stderr)
        sys.exit(0)

if __name__ == "__main__":
    entry_point()