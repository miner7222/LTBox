import re
import subprocess
import time
import serial.tools.list_ports
from pathlib import Path
from typing import Optional, List, Callable, Any, ContextManager

import adbutils
from adbutils import AdbError

from . import constants as const
from . import utils
from .errors import DeviceError, DeviceConnectionError, DeviceCommandError, ToolError
from .i18n import get_string
from .ui import ui

def _wait_loop(
    predicate: Callable[[], Any],
    interval: float = 1.0,
    on_loop: Optional[Callable[[], None]] = None
) -> Any:
    while True:
        res = predicate()
        if res:
            return res
        
        if on_loop:
            on_loop()
            
        time.sleep(interval)

class AdbManager:
    def __init__(self, skip_adb: bool):
        self.skip_adb = skip_adb
        self.connected_once = False
        if const.ADB_EXE.exists():
            adbutils.adb_path = str(const.ADB_EXE)

    def _get_device(self) -> Optional[adbutils.AdbDevice]:
        try:
            return adbutils.adb.device()
        except AdbError:
            return None

    def wait_for_device(self) -> bool:
        if self.skip_adb:
            ui.warn(get_string("device_skip_adb"))
            return False

        if not self.connected_once:
            ui.box_output([
                get_string("device_wait_adb_title"),
                get_string("device_enable_usb_debug"),
                get_string("device_usb_prompt_appear"),
                get_string("device_check_always_allow"),
                get_string("device_wait_cancel_hint")
            ])
        else:
            print(get_string("device_wait_adb_loop") + "...", end="\r")
        
        def _check_adb():
            try:
                for d in adbutils.adb.device_list():
                    if d.get_state() == 'device':
                        return True
            except Exception:
                pass
            return False

        try:
            _wait_loop(_check_adb, interval=1.0)
            
            if not self.connected_once:
                ui.info(get_string("device_adb_connected"))
            self.connected_once = True
            print(" " * 40, end="\r")
            return True
                
        except KeyboardInterrupt:
            ui.warn("\n" + get_string("device_wait_cancelled"))
            self.skip_adb = True
            ui.warn(get_string("act_skip_adb_active"))
            return False

    def get_model(self) -> Optional[str]:
        if not self.wait_for_device():
            return None
        try:
            d = self._get_device()
            return d.prop.model if d else None
        except Exception as e:
            raise DeviceConnectionError(get_string("device_err_get_model").format(e=e), e)

    def get_slot_suffix(self) -> Optional[str]:
        if not self.wait_for_device():
            return None
        try:
            d = self._get_device()
            if d:
                suffix = d.getprop("ro.boot.slot_suffix")
                return suffix if suffix in ["_a", "_b"] else None
            return None
        except Exception as e:
            raise DeviceConnectionError(get_string("device_err_get_slot").format(e=e), e)

    def get_kernel_version(self) -> str:
        if not self.wait_for_device():
             raise DeviceConnectionError(get_string("dl_lkm_kver_fail").format(ver="SKIP_ADB"))
        
        print(get_string("dl_lkm_get_kver"))
        try:
            d = self._get_device()
            if not d:
                raise DeviceConnectionError(get_string("device_err_wait_adb").format(e="No device"))

            version_string = d.shell("cat /proc/version")
            match = re.search(r"Linux version (\d+\.\d+)", version_string)
            if not match:
                raise DeviceCommandError(get_string("dl_lkm_kver_fail").format(ver=version_string))
            
            ver = match.group(1)
            print(get_string("dl_lkm_kver_found").format(ver=ver))
            return ver
        except Exception as e:
             raise DeviceCommandError(get_string("dl_lkm_kver_fail").format(ver=str(e)), e)

    def reboot(self, target: str) -> None:
        if not self.wait_for_device():
            if target == "edl":
                ui.warn(get_string("device_manual_edl_req"))
            return

        try:
            d = self._get_device()
            if d:
                if target == "edl":
                    try:
                        with d.open_transport() as c:
                            c.send_command("reboot:edl")
                            c.check_okay()
                    except Exception:
                        d.shell("reboot edl")
                elif target == "bootloader":
                    try:
                        with d.open_transport() as c:
                            c.send_command("reboot:bootloader")
                            c.check_okay()
                    except Exception:
                        d.shell("reboot bootloader")
                else:
                    d.shell(f"reboot {target}")
        except Exception as e:
            raise DeviceCommandError(get_string("device_err_reboot").format(e=e), e)

    def install(self, apk_path: str) -> None:
        if self.wait_for_device():
            d = self._get_device()
            if d: d.install(apk_path)

    def push(self, local: str, remote: str) -> None:
        if self.wait_for_device():
            d = self._get_device()
            if d: d.sync.push(local, remote)

    def pull(self, remote: str, local: str) -> None:
        if self.wait_for_device():
            d = self._get_device()
            if d: d.sync.pull(remote, local)

    def shell(self, cmd: str) -> str:
        if self.wait_for_device():
            d = self._get_device()
            if d: return d.shell(cmd)
        return ""

    def force_kill_server(self):
        try:
            subprocess.run(
                ["taskkill", "/F", "/IM", "adb.exe", "/T"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0
            )
        except Exception:
            pass

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
            raise DeviceCommandError(get_string("device_err_get_slot_fastboot").format(e=e), e)

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
        
        def _loop_msg():
            ui.info(get_string("device_wait_fastboot_loop"))

        try:
            _wait_loop(lambda: self.check_device(silent=True), interval=2.0, on_loop=_loop_msg)
            ui.info(get_string("device_fastboot_connected"))
            return True
        except KeyboardInterrupt:
            ui.warn(get_string("device_wait_fastboot_cancel"))
            raise

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
        
        def _loop_msg():
            ui.info(get_string("device_wait_edl_loop"))

        try:
            port_name = _wait_loop(lambda: self.check_device(silent=True), interval=2.0, on_loop=_loop_msg)
            ui.info(get_string("device_edl_connected").format(port=port_name))
            return port_name
        except KeyboardInterrupt:
            ui.warn(get_string("device_wait_edl_cancel"))
            raise

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
            raise DeviceCommandError(msg, e)

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
            raise DeviceCommandError(get_string("device_err_fh_exec").format(e=e), e)

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
            raise DeviceCommandError(get_string("device_err_flash_exec").format(e=e), e)

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
            raise DeviceCommandError(get_string("device_err_reset_fail").format(e=e), e)

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
            raise DeviceCommandError(get_string("device_err_rawprogram_fail").format(e=e), e)

class DeviceController:
    def __init__(self, skip_adb: bool = False):
        self._skip_adb = skip_adb
        self.adb = AdbManager(skip_adb)
        self.fastboot = FastbootManager()
        self.edl = EdlManager()

    @property
    def skip_adb(self) -> bool:
        return self.adb.skip_adb

    @skip_adb.setter
    def skip_adb(self, value: bool) -> None:
        self._skip_adb = value
        self.adb.skip_adb = value

    def detect_active_slot(self) -> Optional[str]:
        slot = self.adb.get_slot_suffix()
        if slot:
            return slot
            
        ui.echo("\n" + "="*60)
        ui.echo(get_string("act_manual_fastboot"))
        ui.echo("="*60 + "\n")
        
        self.ensure_fastboot_mode()
        return self.fastboot.get_slot_suffix()

    def ensure_fastboot_mode(self) -> None:
        if self.fastboot.check_device(silent=True):
            return

        if not self.skip_adb:
            try:
                self.adb.reboot("bootloader")
            except Exception as e:
                ui.warn(get_string("act_err_reboot_bl").format(e=e))
        
        self.fastboot.wait_for_device()

    def ensure_edl_mode(self) -> None:
        if self.edl.check_device(silent=True):
            ui.info(get_string("device_already_edl"))
            return

        if not self.skip_adb:
            self.adb.wait_for_device()
            ui.info(get_string("device_edl_setup_title"))
            self.adb.reboot("edl")
            ui.info(get_string("device_wait_10s_edl"))
            time.sleep(10)
        else:
            ui.echo("\n" + "="*60)
            ui.echo(get_string("act_manual_edl"))
            ui.echo("="*60 + "\n")

    def setup_edl_connection(self) -> str:
        self.ensure_edl_mode()
        
        ui.info(get_string("device_wait_loader_title"))
        required_files = [const.EDL_LOADER_FILENAME]
        prompt = get_string("device_loader_prompt").format(loader=const.EDL_LOADER_FILENAME, folder=const.IMAGE_DIR.name)
        
        const.IMAGE_DIR.mkdir(exist_ok=True)
        utils.wait_for_files(const.IMAGE_DIR, required_files, prompt)
        ui.info(get_string("device_loader_found").format(file=const.EDL_LOADER_FILE.name, dir=const.IMAGE_DIR.name))

        port = self.edl.wait_for_device()
        ui.info(get_string("device_edl_setup_done"))
        return port
