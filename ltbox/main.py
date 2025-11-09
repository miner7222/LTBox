import argparse
import os
import platform
import subprocess
import sys
from pathlib import Path
from datetime import datetime

sys.path.insert(0, str(Path(__file__).parent.parent.resolve()))

try:
    from ltbox import utils, actions, workflow
except ImportError as e:
    print(f"[!] Error: Failed to import 'ltbox' package.", file=sys.stderr)
    print(f"[!] Details: {e}", file=sys.stderr)
    print(f"[!] Please ensure the 'ltbox' folder and its files are present.", file=sys.stderr)
    if platform.system() == "Windows":
        os.system("pause")
    sys.exit(1)

class Tee:
    def __init__(self, original_stream, log_file):
        self.original_stream = original_stream
        self.log_file = log_file

    def write(self, message):
        try:
            self.original_stream.write(message)
            self.log_file.write(message)
        except Exception as e:
            self.original_stream.write(f"\n[!] Logging Error: {e}\n")

    def flush(self):
        try:
            self.original_stream.flush()
            self.log_file.flush()
        except Exception:
            pass

def main():
    parser = argparse.ArgumentParser(description="Android Image Patcher and AVB Tool.")
    subparsers = parser.add_subparsers(dest="command", required=True, help="Available commands")

    subparsers.add_parser("convert", help="Convert vendor_boot region and remake vbmeta.")
    subparsers.add_parser("root_device", help="Root the device via EDL.")
    subparsers.add_parser("unroot_device", help="Unroot the device via EDL.")
    subparsers.add_parser("disable_ota", help="Disable OTA updates via ADB.")
    subparsers.add_parser("edit_dp", help="Edit devinfo and persist images.")
    subparsers.add_parser("read_edl", help="Read devinfo and persist images via EDL.")
    subparsers.add_parser("write_edl", help="Write patched devinfo and persist images via EDL.")
    subparsers.add_parser("read_anti_rollback", help="Read and Compare Anti-Rollback indices.")
    subparsers.add_parser("patch_anti_rollback", help="Patch firmware images to bypass Anti-Rollback.")
    subparsers.add_parser("write_anti_rollback", help="Flash patched Anti-Rollback images via EDL.")
    subparsers.add_parser("clean", help="Remove downloaded tools, I/O folders, and temp files.")
    
    subparsers.add_parser("modify_xml_wipe", help="Modify XML files from RSA firmware for flashing (WIPE DATA).")
    subparsers.add_parser("modify_xml", help="Modify XML files from RSA firmware for flashing (NO WIPE).")
    
    subparsers.add_parser("flash_edl", help="Flash the entire modified firmware via EDL.")
    subparsers.add_parser("patch_all", help="Run the full automated ROW flashing process (NO WIPE).")
    subparsers.add_parser("patch_all_wipe", help="Run the full automated ROW flashing process (WIPE DATA).")
    parser_info = subparsers.add_parser("info", help="Display AVB info for image files or directories.")
    parser_info.add_argument("files", nargs='+', help="Image file(s) or folder(s) to inspect.")
    
    subparsers.add_parser("root", help="Patch boot.img with KernelSU (offline).")


    args = parser.parse_args()
    
    log_file_handle = None
    original_stdout = sys.stdout
    original_stderr = sys.stderr

    if args.command in ["patch_all", "patch_all_wipe"]:
        try:
            timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
            log_filename = f"log_{timestamp}.txt"
            
            log_file_handle = open(log_filename, 'w', encoding='utf-8')
            
            sys.stdout = Tee(original_stdout, log_file_handle)
            sys.stderr = Tee(original_stderr, log_file_handle)
            
            print(f"--- Logging enabled. Output will be saved to {log_filename} ---")
            print(f"--- Command: {args.command} ---")
        except Exception as e:
            sys.stdout = original_stdout
            sys.stderr = original_stderr
            if log_file_handle:
                log_file_handle.close()
            print(f"[!] Failed to initialize logger: {e}", file=sys.stderr)
            log_file_handle = None
    
    skip_adb = os.environ.get('SKIP_ADB', '0') == '1'

    try:
        if args.command == "convert":
            actions.convert_images()
        elif args.command == "root_device":
            actions.root_device(skip_adb=skip_adb)
        elif args.command == "unroot_device":
            actions.unroot_device(skip_adb=skip_adb)
        elif args.command == "root":
            actions.root_boot_only()
        elif args.command == "disable_ota":
            actions.disable_ota(skip_adb=skip_adb)
        elif args.command == "edit_dp":
            actions.edit_devinfo_persist()
        elif args.command == "read_edl":
            actions.read_edl(skip_adb=skip_adb)
        elif args.command == "write_edl":
            actions.write_edl()
        elif args.command == "read_anti_rollback":
            actions.read_anti_rollback()
        elif args.command == "patch_anti_rollback":
            actions.patch_anti_rollback()
        elif args.command == "write_anti_rollback":
            actions.write_anti_rollback()
        elif args.command == "clean":
            utils.clean_workspace()
        elif args.command == "modify_xml":
            actions.modify_xml(wipe=0)
        elif args.command == "modify_xml_wipe":
            actions.modify_xml(wipe=1)
        elif args.command == "flash_edl":
            actions.flash_edl()
        elif args.command == "patch_all":
            workflow.patch_all(wipe=0, skip_adb=skip_adb)
        elif args.command == "patch_all_wipe":
            workflow.patch_all(wipe=1, skip_adb=skip_adb)
        elif args.command == "info":
            utils.show_image_info(args.files)
    except (subprocess.CalledProcessError, FileNotFoundError, RuntimeError, KeyError) as e:
        if not isinstance(e, SystemExit):
            print(f"\nAn unexpected error occurred: {e}", file=sys.stderr)
    except SystemExit:
        print("\nProcess halted by script (e.g., file not found).", file=sys.stderr)
    except KeyboardInterrupt:
        print("\nProcess cancelled by user.", file=sys.stderr)


    finally:
        print()
        
        if log_file_handle:
            print(f"--- Logging finished. Output saved to {log_file_handle.name} ---")
            sys.stdout = original_stdout
            sys.stderr = original_stderr
            log_file_handle.close()

        if platform.system() == "Windows":
            os.system("pause")

if __name__ == "__main__":
    main()