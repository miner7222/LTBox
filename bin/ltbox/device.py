import os
import re
import subprocess
import sys
import time
import serial.tools.list_ports
from pathlib import Path
from typing import Optional, List, Dict

from . import constants as const
from . import utils
from .i18n import get_string

class DeviceController:
    def __init__(self, skip_adb: bool = False):
        self.skip_adb = skip_adb
        self.edl_port: Optional[str] = None
        self.adb_connected_once: bool = False

    def wait_for_adb(self) -> None:
        if self.skip_adb:
            print(get_string("device_skip_adb"))
            return
        
        if not self.adb_connected_once:
            print(get_string("device_wait_adb_title"))
            print(get_string("device_enable_usb_debug"))
            print(get_string("device_usb_prompt_appear"))
            print(get_string("device_check_always_allow"))
        
        try:
            utils.run_command([str(const.ADB_EXE), "wait-for-device"])
            print(get_string("device_adb_connected"))
            self.adb_connected_once = True
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_wait_adb").format(e=e))

    def get_device_model(self) -> Optional[str]:
        self.wait_for_adb()
        if self.skip_adb:
            return None
        try:
            result = utils.run_command([str(const.ADB_EXE), "shell", "getprop", "ro.product.model"], capture=True)
            model = result.stdout.strip()
            if not model:
                return None
            return model
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_get_model").format(e=e))

    def get_active_slot_suffix(self) -> Optional[str]:
        self.wait_for_adb()
        if self.skip_adb:
            return None
        try:
            result = utils.run_command([str(const.ADB_EXE), "shell", "getprop", "ro.boot.slot_suffix"], capture=True)
            suffix = result.stdout.strip()
            if suffix not in ["_a", "_b"]:
                return None
            return suffix
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_get_slot").format(e=e))

    def get_active_slot_suffix_from_fastboot(self) -> Optional[str]:
        try:
            result = utils.run_command([str(const.FASTBOOT_EXE), "getvar", "current-slot"], capture=True, check=False)
            output = result.stderr.strip() + "\n" + result.stdout.strip()
            
            match = re.search(r"current-slot:\s*([a-z]+)", output)
            if match:
                slot = match.group(1).strip()
                if slot in ['a', 'b']:
                    suffix = f"_{slot}"
                    return suffix
            
            print(get_string("device_warn_slot_fastboot").format(snippet=output.splitlines()[0] if output else 'None'))
            return None
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_get_slot_fastboot").format(e=e))

    def reboot_to_edl(self) -> None:
        self.wait_for_adb()
        if self.skip_adb:
            print(get_string("device_manual_edl_req"))
            return
        try:
            utils.run_command([str(const.ADB_EXE), "reboot", "edl"])
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_reboot").format(e=e))

    def reboot_to_bootloader(self) -> None:
        self.wait_for_adb()
        if self.skip_adb:
            return
        try:
            utils.run_command([str(const.ADB_EXE), "reboot", "bootloader"])
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_reboot").format(e=e))

    def check_fastboot_device(self, silent: bool = False) -> bool:
        if not silent:
            print(get_string("device_check_fastboot"))
        try:
            result = utils.run_command([str(const.FASTBOOT_EXE), "devices"], capture=True, check=False)
            output = result.stdout.strip()
            
            if output:
                if not silent:
                    print(get_string("device_found_fastboot").format(output=output))
                return True
            
            if not silent:
                print(get_string("device_no_fastboot"))
                print(get_string("device_connect_fastboot"))
            return False
        
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            if not silent:
                print(get_string("device_err_check_fastboot").format(e=e), file=sys.stderr)
            return False

    def wait_for_fastboot(self) -> bool:
        print(get_string("device_wait_fastboot_title"))
        if self.check_fastboot_device(silent=True):
            print(get_string("device_fastboot_connected"))
            return True
        
        while not self.check_fastboot_device(silent=True):
            print(get_string("device_wait_fastboot_loop"))
            try:
                time.sleep(2)
            except KeyboardInterrupt:
                print(get_string("device_wait_fastboot_cancel"))
                raise
        print(get_string("device_fastboot_connected"))
        return True

    def fastboot_reboot_system(self) -> None:
        try:
            utils.run_command([str(const.FASTBOOT_EXE), "reboot"])
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_reboot").format(e=e))
            
    def get_fastboot_vars(self) -> str:
        print(get_string("device_rollback_header"))

        if not self.skip_adb:
            print(get_string("device_rebooting_fastboot"))
            self.reboot_to_bootloader()
            print(get_string("device_wait_10s_fastboot"))
            time.sleep(10)
        else:
            print(get_string("device_skip_adb_on"))
            print(get_string("device_manual_reboot_fastboot"))
            print(get_string("device_press_enter_fastboot"))
            try:
                input()
            except EOFError:
                pass
        
        self.wait_for_fastboot()
        
        print(get_string("device_read_rollback"))
        try:
            result = utils.run_command([str(const.FASTBOOT_EXE), "getvar", "all"], capture=True, check=False)
            output = result.stdout + "\n" + result.stderr
            
            if not self.skip_adb:
                print(get_string("device_reboot_back_sys"))
                self.fastboot_reboot_system()
            else:
                print(get_string("device_skip_adb_leave_fastboot"))
                print(get_string("device_manual_next_steps"))
            
            return output
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            print(get_string("device_err_fastboot_vars").format(e=e), file=sys.stderr)
            
            if not self.skip_adb:
                print(get_string("device_attempt_reboot_sys"))
                try:
                    self.fastboot_reboot_system()
                except (ToolError, subprocess.CalledProcessError, FileNotFoundError):
                    pass
            raise ToolError(get_string("device_err_fastboot_vars").format(e=e))

    def check_edl_device(self, silent: bool = False) -> Optional[str]:
        if not silent:
            print(get_string("device_check_edl"))
        
        try:
            ports = serial.tools.list_ports.comports()
            for port in ports:
                is_qualcomm_port = (port.description and "Qualcomm" in port.description and "9008" in port.description) or \
                                   (port.hwid and "VID:PID=05C6:9008" in port.hwid.upper())
                
                if is_qualcomm_port:
                    if not silent:
                        print(get_string("device_found_edl").format(device=port.device))
                    return port.device
            
            if not silent:
                print(get_string("device_no_edl"))
                print(get_string("device_connect_edl"))
            return None
        
        except serial.SerialException as e:
            if not silent:
                print(get_string("device_err_check_edl").format(e=e), file=sys.stderr)
            return None

    def wait_for_edl(self) -> str:
        print(get_string("device_wait_edl_title"))
        port_name = self.check_edl_device()
        if port_name:
            return port_name
        
        while not (port_name := self.check_edl_device(silent=True)):
            print(get_string("device_wait_edl_loop"))
            try:
                time.sleep(2)
            except KeyboardInterrupt:
                print(get_string("device_wait_edl_cancel"))
                raise
        print(get_string("device_edl_connected").format(port=port_name))
        return port_name

    def setup_edl_connection(self) -> str:
        if self.check_edl_device(silent=True):
            print(get_string("device_already_edl"))
        else:
            if not self.skip_adb:
                self.wait_for_adb()
            
            print(get_string("device_edl_setup_title"))
            self.reboot_to_edl()
            
            if not self.skip_adb:
                print(get_string("device_wait_10s_edl"))
                time.sleep(10)

        print(get_string("device_wait_loader_title"))
        required_files = [const.EDL_LOADER_FILENAME]
        prompt = get_string("device_loader_prompt").format(loader=const.EDL_LOADER_FILENAME, folder=const.IMAGE_DIR.name)
        
        const.IMAGE_DIR.mkdir(exist_ok=True)
        utils.wait_for_files(const.IMAGE_DIR, required_files, prompt)
        print(get_string("device_loader_found").format(file=const.EDL_LOADER_FILE.name, dir=const.IMAGE_DIR.name))

        port = self.wait_for_edl()
        self.edl_port = port
        print(get_string("device_edl_setup_done"))
        return port

    def load_firehose_programmer(self, loader_path: Path, port: str) -> None:
        if not const.QSAHARASERVER_EXE.exists():
            raise FileNotFoundError(get_string("device_err_qsahara_missing").format(path=const.QSAHARASERVER_EXE))
            
        port_str = f"\\\\.\\{port}"
        
        cmd_sahara = [
            str(const.QSAHARASERVER_EXE),
            "-p", port_str,
            "-s", f"13:{loader_path}"
        ]
        
        try:
            utils.run_command(cmd_sahara, check=True)
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            msg = get_string("device_fatal_programmer")
            msg += f"\n{get_string('device_fatal_causes')}"
            msg += f"\n{get_string('device_cause_1')}"
            msg += f"\n{get_string('device_cause_2')}"
            msg += f"\n{get_string('device_cause_3')}"
            msg += f"\nError: {e}"
            raise ToolError(msg)

    def load_firehose_programmer_with_stability(self, loader_path: Path, port: str) -> None:
        print(get_string("device_upload_programmer").format(port=port))
        self.load_firehose_programmer(loader_path, port)
        time.sleep(2)

    def fh_loader_read_part(
        self,
        port: str, 
        output_filename: str, 
        lun: str, 
        start_sector: str, 
        num_sectors: str, 
        memory_name: str = "UFS"
    ) -> None:
        if not const.FH_LOADER_EXE.exists():
            raise FileNotFoundError(get_string("device_err_fh_missing").format(path=const.FH_LOADER_EXE))

        dest_file = Path(output_filename).resolve()
        dest_dir = dest_file.parent
        dest_filename = dest_file.name
        
        dest_dir.mkdir(parents=True, exist_ok=True)

        port_str = f"\\\\.\\{port}"
        cmd_fh = [
            str(const.FH_LOADER_EXE),
            f"--port={port_str}",
            "--convertprogram2read",
            f"--sendimage={dest_filename}",
            f"--lun={lun}",
            f"--start_sector={start_sector}",
            f"--num_sectors={num_sectors}",
            f"--memoryname={memory_name}",
            "--noprompt",
            "--zlpawarehost=1"
        ]
        
        try:
            utils.run_command(cmd_fh, cwd=dest_dir, check=True)
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_fh_exec").format(e=e))

    def fh_loader_write_part(
        self,
        port: str, 
        image_path: Path, 
        lun: str, 
        start_sector: str, 
        memory_name: str = "UFS"
    ) -> None:
        if not const.FH_LOADER_EXE.exists():
            raise FileNotFoundError(get_string("device_err_fh_missing").format(path=const.FH_LOADER_EXE))

        image_file = Path(image_path).resolve()
        work_dir = image_file.parent
        filename = image_file.name
        
        port_str = f"\\\\.\\{port}"
        
        cmd_fh = [
            str(const.FH_LOADER_EXE),
            f"--port={port_str}",
            f"--sendimage={filename}",
            f"--lun={lun}",
            f"--start_sector={start_sector}",
            f"--memoryname={memory_name}",
            "--noprompt",
            "--zlpawarehost=1"
        ]
        
        try:
            utils.run_command(cmd_fh, cwd=work_dir, check=True)
            print(get_string("device_flash_success").format(filename=filename))
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_flash_exec").format(e=e))

    def fh_loader_reset(self, port: str) -> None:
        if not const.FH_LOADER_EXE.exists():
            raise FileNotFoundError(get_string("device_err_fh_missing").format(path=const.FH_LOADER_EXE))
            
        port_str = f"\\\\.\\{port}"
        
        cmd_fh = [
            str(const.FH_LOADER_EXE),
            f"--port={port_str}",
            "--reset",
            "--noprompt"
        ]
        try:
            utils.run_command(cmd_fh)
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(f"Failed to reset device: {e}")

    def edl_rawprogram(
        self,
        loader_path: Path, 
        memory_type: str, 
        raw_xmls: List[Path], 
        patch_xmls: List[Path], 
        port: str
    ) -> None:
        if not const.QSAHARASERVER_EXE.exists() or not const.FH_LOADER_EXE.exists():
            print(get_string("device_err_tools_missing").format(dir=const.TOOLS_DIR.name))
            raise FileNotFoundError(get_string("device_err_edl_tools_missing"))
        
        port_str = f"\\\\.\\{port}"
        search_path = str(loader_path.parent)

        print(get_string("device_step1_load"))
        self.load_firehose_programmer_with_stability(loader_path, port)

        print(get_string("device_step2_flash"))
        raw_xml_str = ",".join([p.name for p in raw_xmls])
        patch_xml_str = ",".join([p.name for p in patch_xmls])

        cmd_fh = [
            str(const.FH_LOADER_EXE),
            f"--port={port_str}",
            f"--search_path={search_path}",
            f"--sendxml={raw_xml_str}",
            f"--sendxml={patch_xml_str}",
            "--setactivepartition=1",
            f"--memoryname={memory_type}",
            "--showpercentagecomplete",
            "--zlpawarehost=1",
            "--noprompt"
        ]
        
        try:
            utils.run_command(cmd_fh)
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(f"EDL Rawprogram flash failed: {e}")