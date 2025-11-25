import os
import re
import subprocess
import sys
import time
import serial.tools.list_ports
from pathlib import Path
from typing import Optional, List

from . import constants as const
from . import utils
from .errors import ToolError
from .i18n import get_string
from .utils import ui

class AdbManager:
    def __init__(self, skip_adb: bool):
        self.skip_adb = skip_adb
        self.connected_once = False

    def wait_for_device(self) -> None:
        if self.skip_adb:
            ui.warn(get_string("device_skip_adb"))
            return
        
        if not self.connected_once:
            ui.box_output([
                get_string("device_wait_adb_title"),
                get_string("device_enable_usb_debug"),
                get_string("device_usb_prompt_appear"),
                get_string("device_check_always_allow")
            ])
        
        try:
            utils.run_command([str(const.ADB_EXE), "wait-for-device"])
            if not self.connected_once:
                ui.info(get_string("device_adb_connected"))
            self.connected_once = True
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_wait_adb").format(e=e))

    def get_model(self) -> Optional[str]:
        self.wait_for_device()
        if self.skip_adb:
            return None
        try:
            result = utils.run_command([str(const.ADB_EXE), "shell", "getprop", "ro.product.model"], capture=True)
            model = result.stdout.strip()
            return model if model else None
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_get_model").format(e=e))

    def get_slot_suffix(self) -> Optional[str]:
        self.wait_for_device()
        if self.skip_adb:
            return None
        try:
            result = utils.run_command([str(const.ADB_EXE), "shell", "getprop", "ro.boot.slot_suffix"], capture=True)
            suffix = result.stdout.strip()
            return suffix if suffix in ["_a", "_b"] else None
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_get_slot").format(e=e))

    def get_kernel_version(self) -> str:
        self.wait_for_device()
        if self.skip_adb:
             raise ToolError(get_string("dl_lkm_kver_fail").format(ver="SKIP_ADB"))
        
        print(get_string("dl_lkm_get_kver"))
        try:
            result = utils.run_command(
                [str(const.ADB_EXE), "shell", "cat", "/proc/version"],
                capture=True,
                check=True
            )
            version_string = result.stdout.strip()
            
            match = re.search(r"Linux version (\d+\.\d+)", version_string)
            if not match:
                raise ToolError(get_string("dl_lkm_kver_fail").format(ver=version_string))
            
            ver = match.group(1)
            print(get_string("dl_lkm_kver_found").format(ver=ver))
            return ver
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
             raise ToolError(get_string("dl_lkm_kver_fail").format(ver=str(e)))

    def reboot_edl(self) -> None:
        self.wait_for_device()
        if self.skip_adb:
            ui.warn(get_string("device_manual_edl_req"))
            return
        try:
            utils.run_command([str(const.ADB_EXE), "reboot", "edl"])
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_reboot").format(e=e))

    def reboot_bootloader(self) -> None:
        self.wait_for_device()
        if self.skip_adb:
            return
        try:
            utils.run_command([str(const.ADB_EXE), "reboot", "bootloader"])
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_reboot").format(e=e))

class FastbootManager:
    def get_slot_suffix(self) -> Optional[str]:
        try:
            result = utils.run_command([str(const.FASTBOOT_EXE), "getvar", "current-slot"], capture=True, check=False)
            output = result.stderr.strip() + "\n" + result.stdout.strip()
            
            match = re.search(r"current-slot:\s*([a-z]+)", output)
            if match:
                slot = match.group(1).strip()
                if slot in ['a', 'b']:
                    return f"_{slot}"
            
            ui.warn(get_string("device_warn_slot_fastboot").format(snippet=output.splitlines()[0] if output else 'None'))
            return None
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_get_slot_fastboot").format(e=e))

    def check_device(self, silent: bool = False) -> bool:
        if not silent:
            ui.info(get_string("device_check_fastboot"))
        try:
            result = utils.run_command([str(const.FASTBOOT_EXE), "devices"], capture=True, check=False)
            output = result.stdout.strip()
            
            if output:
                if not silent:
                    ui.info(get_string("device_found_fastboot").format(output=output))
                return True
            
            if not silent:
                ui.warn(get_string("device_no_fastboot"))
                ui.warn(get_string("device_connect_fastboot"))
            return False
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            if not silent:
                ui.error(get_string("device_err_check_fastboot").format(e=e))
            return False

    def wait_for_device(self) -> bool:
        ui.info(get_string("device_wait_fastboot_title"))
        if self.check_device(silent=True):
            ui.info(get_string("device_fastboot_connected"))
            return True
        
        while not self.check_device(silent=True):
            ui.info(get_string("device_wait_fastboot_loop"))
            try:
                time.sleep(2)
            except KeyboardInterrupt:
                ui.warn(get_string("device_wait_fastboot_cancel"))
                raise
        ui.info(get_string("device_fastboot_connected"))
        return True

    def reboot_system(self) -> None:
        try:
            utils.run_command([str(const.FASTBOOT_EXE), "reboot"])
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_reboot").format(e=e))

class EdlManager:
    def check_device(self, silent: bool = False) -> Optional[str]:
        if not silent:
            ui.info(get_string("device_check_edl"))
        
        try:
            ports = serial.tools.list_ports.comports()
            for port in ports:
                is_qualcomm_port = (port.description and "Qualcomm" in port.description and "9008" in port.description) or \
                                   (port.hwid and "VID:PID=05C6:9008" in port.hwid.upper())
                
                if is_qualcomm_port:
                    if not silent:
                        ui.info(get_string("device_found_edl").format(device=port.device))
                    return port.device
            
            if not silent:
                ui.warn(get_string("device_no_edl"))
                ui.warn(get_string("device_connect_edl"))
            return None
        except serial.SerialException as e:
            if not silent:
                ui.error(get_string("device_err_check_edl").format(e=e))
            return None

    def wait_for_device(self) -> str:
        ui.info(get_string("device_wait_edl_title"))
        port_name = self.check_device()
        if port_name:
            return port_name
        
        while not (port_name := self.check_device(silent=True)):
            ui.info(get_string("device_wait_edl_loop"))
            try:
                time.sleep(2)
            except KeyboardInterrupt:
                ui.warn(get_string("device_wait_edl_cancel"))
                raise
        ui.info(get_string("device_edl_connected").format(port=port_name))
        return port_name

    def load_programmer(self, port: str, loader_path: Path) -> None:
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

    def load_programmer_safe(self, port: str, loader_path: Path) -> None:
        ui.info(get_string("device_upload_programmer").format(port=port))
        self.load_programmer(port, loader_path)
        time.sleep(2)

    def read_partition(self, port: str, output_filename: str, lun: str, start_sector: str, num_sectors: str, memory_name: str = "UFS") -> None:
        if not const.EDL_EXE.exists():
            raise FileNotFoundError(get_string("device_err_fh_missing").format(path=const.EDL_EXE))

        dest_file = Path(output_filename).resolve()
        dest_dir = dest_file.parent
        dest_filename = dest_file.name
        
        dest_dir.mkdir(parents=True, exist_ok=True)

        port_str = f"\\\\.\\{port}"
        cmd_fh = [
            str(const.EDL_EXE),
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

    def write_partition(self, port: str, image_path: Path, lun: str, start_sector: str, memory_name: str = "UFS") -> None:
        if not const.EDL_EXE.exists():
            raise FileNotFoundError(get_string("device_err_fh_missing").format(path=const.EDL_EXE))

        image_file = Path(image_path).resolve()
        work_dir = image_file.parent
        filename = image_file.name
        
        port_str = f"\\\\.\\{port}"
        
        cmd_fh = [
            str(const.EDL_EXE),
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
            ui.info(get_string("device_flash_success").format(filename=filename))
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(get_string("device_err_flash_exec").format(e=e))

    def reset(self, port: str) -> None:
        if not const.EDL_EXE.exists():
            raise FileNotFoundError(get_string("device_err_fh_missing").format(path=const.EDL_EXE))
            
        port_str = f"\\\\.\\{port}"
        
        cmd_fh = [
            str(const.EDL_EXE),
            f"--port={port_str}",
            "--reset",
            "--noprompt"
        ]
        try:
            utils.run_command(cmd_fh)
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise ToolError(f"Failed to reset device: {e}")

    def flash_rawprogram(self, port: str, loader_path: Path, memory_type: str, raw_xmls: List[Path], patch_xmls: List[Path]) -> None:
        if not const.QSAHARASERVER_EXE.exists() or not const.EDL_EXE.exists():
            ui.error(get_string("device_err_tools_missing").format(dir=const.TOOLS_DIR.name))
            raise FileNotFoundError(get_string("device_err_edl_tools_missing"))
        
        port_str = f"\\\\.\\{port}"
        search_path = str(loader_path.parent)

        ui.info(get_string("device_step1_load"))
        self.load_programmer_safe(port, loader_path)

        ui.info(get_string("device_step2_flash"))
        raw_xml_str = ",".join([p.name for p in raw_xmls])
        patch_xml_str = ",".join([p.name for p in patch_xmls])

        cmd_fh = [
            str(const.EDL_EXE),
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

class DeviceController:
    def __init__(self, skip_adb: bool = False):
        self._skip_adb = skip_adb
        self.adb = AdbManager(skip_adb)
        self.fastboot = FastbootManager()
        self.edl = EdlManager()

    @property
    def skip_adb(self) -> bool:
        return self._skip_adb

    @skip_adb.setter
    def skip_adb(self, value: bool) -> None:
        self._skip_adb = value
        self.adb.skip_adb = value

    def wait_for_adb(self) -> None:
        self.adb.wait_for_device()

    def get_device_model(self) -> Optional[str]:
        return self.adb.get_model()

    def get_active_slot_suffix(self) -> Optional[str]:
        return self.adb.get_slot_suffix()

    def get_active_slot_suffix_from_fastboot(self) -> Optional[str]:
        return self.fastboot.get_slot_suffix()
        
    def get_kernel_version(self) -> str:
        return self.adb.get_kernel_version()

    def reboot_to_edl(self) -> None:
        self.adb.reboot_edl()

    def reboot_to_bootloader(self) -> None:
        self.adb.reboot_bootloader()

    def check_fastboot_device(self, silent: bool = False) -> bool:
        return self.fastboot.check_device(silent)

    def wait_for_fastboot(self) -> bool:
        return self.fastboot.wait_for_device()

    def fastboot_reboot_system(self) -> None:
        self.fastboot.reboot_system()

    def check_edl_device(self, silent: bool = False) -> Optional[str]:
        return self.edl.check_device(silent)

    def wait_for_edl(self) -> str:
        return self.edl.wait_for_device()

    def setup_edl_connection(self) -> str:
        if self.edl.check_device(silent=True):
            ui.info(get_string("device_already_edl"))
        else:
            if not self.skip_adb:
                self.adb.wait_for_device()
            
            ui.info(get_string("device_edl_setup_title"))
            self.adb.reboot_edl()
            
            if not self.skip_adb:
                ui.info(get_string("device_wait_10s_edl"))
                time.sleep(10)

        ui.info(get_string("device_wait_loader_title"))
        required_files = [const.EDL_LOADER_FILENAME]
        prompt = get_string("device_loader_prompt").format(loader=const.EDL_LOADER_FILENAME, folder=const.IMAGE_DIR.name)
        
        const.IMAGE_DIR.mkdir(exist_ok=True)
        utils.wait_for_files(const.IMAGE_DIR, required_files, prompt)
        ui.info(get_string("device_loader_found").format(file=const.EDL_LOADER_FILE.name, dir=const.IMAGE_DIR.name))

        port = self.edl.wait_for_device()
        ui.info(get_string("device_edl_setup_done"))
        return port

    def load_firehose_programmer(self, loader_path: Path, port: str) -> None:
        self.edl.load_programmer(port, loader_path)

    def load_firehose_programmer_with_stability(self, loader_path: Path, port: str) -> None:
        self.edl.load_programmer_safe(port, loader_path)

    def edl_read_partition(self, port: str, output_filename: str, lun: str, start_sector: str, num_sectors: str, memory_name: str = "UFS") -> None:
        self.edl.read_partition(port, output_filename, lun, start_sector, num_sectors, memory_name)

    def edl_write_partition(self, port: str, image_path: Path, lun: str, start_sector: str, memory_name: str = "UFS") -> None:
        self.edl.write_partition(port, image_path, lun, start_sector, memory_name)

    def edl_reset(self, port: str) -> None:
        self.edl.reset(port)

    def edl_rawprogram(self, loader_path: Path, memory_type: str, raw_xmls: List[Path], patch_xmls: List[Path], port: str) -> None:
        self.edl.flash_rawprogram(port, loader_path, memory_type, raw_xmls, patch_xmls)