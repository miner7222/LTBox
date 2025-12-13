import re
import shutil
import sys
from pathlib import Path
from typing import Dict, Optional, Union

from .. import constants as const
from .. import utils, downloader, device
from ..i18n import get_string

def patch_boot_with_root_algo(
    work_dir: Path, 
    magiskboot_exe: Path, 
    dev: Optional[device.DeviceController] = None, 
    gki: bool = False,
    lkm_kernel_version: Optional[str] = None,
    root_type: str = "ksu"
) -> Optional[Path]:
    
    img_name = const.FN_BOOT if gki else const.FN_INIT_BOOT
    out_img_name = const.FN_BOOT_ROOT if gki else const.FN_INIT_BOOT_ROOT
    
    patched_boot_path = const.BASE_DIR / out_img_name
    work_img_path = work_dir / img_name

    if not work_img_path.exists():
        print(get_string("img_root_err_img_not_found").format(name=img_name), file=sys.stderr)
        return None

    if gki:
        print(get_string("img_root_step1").format(name=img_name))
        utils.run_command([str(magiskboot_exe), "unpack", img_name], cwd=work_dir)
        if not (work_dir / "kernel").exists():
            print(get_string("img_root_unpack_fail"))
            return None
        print(get_string("img_root_unpack_ok"))

        print(get_string("img_root_step2"))
        target_kernel_version = get_kernel_version(work_dir / "kernel")

        if not target_kernel_version:
                print(get_string("img_root_kernel_ver_fail"))
                return None

        if not re.match(r"\d+\.\d+\.\d+", target_kernel_version):
                print(get_string("img_root_kernel_invalid").format(ver=target_kernel_version))
                return None
        
        print(get_string("img_root_target_ver").format(ver=target_kernel_version))

        kernel_image_path = downloader.get_gki_kernel(target_kernel_version, work_dir)

        print(get_string("img_root_step5"))
        shutil.move(str(kernel_image_path), work_dir / "kernel")
        print(get_string("img_root_kernel_replaced"))

        print(get_string("img_root_step6").format(name=img_name))
        utils.run_command([str(magiskboot_exe), "repack", img_name], cwd=work_dir)
        if not (work_dir / "new-boot.img").exists():
            print(get_string("img_root_repack_fail"))
            return None
        shutil.move(work_dir / "new-boot.img", patched_boot_path)
        print(get_string("img_root_repack_ok"))

        downloader.download_ksu_apk(const.BASE_DIR)
        
        return patched_boot_path
    
    else:
        print(get_string("img_root_step1_init_boot").format(name=img_name))
        utils.run_command([str(magiskboot_exe), "unpack", img_name], cwd=work_dir)
        if not (work_dir / "ramdisk.cpio").exists():
            print(get_string("img_root_unpack_fail"))
            return None
        print(get_string("img_root_unpack_ok"))

        print(get_string("img_root_lkm_download"))
        try:
            ksuinit_path = work_dir / "init"
            kmod_path = work_dir / "kernelsu.ko"
            
            if root_type == "sukisu":
                if not lkm_kernel_version:
                     print(get_string("img_root_lkm_no_dev"), file=sys.stderr)
                     return None
                
                downloader.download_sukisu_init(ksuinit_path)
                downloader.get_sukisu_lkm(kmod_path, lkm_kernel_version)
            else:
                downloader.download_ksuinit(ksuinit_path)
                if not lkm_kernel_version:
                     print(get_string("img_root_lkm_no_dev"), file=sys.stderr)
                     return None
                downloader.get_lkm_kernel(kmod_path, lkm_kernel_version)

        except Exception as e:
            print(get_string("img_root_lkm_download_fail").format(e=e), file=sys.stderr)
            return None

        print(get_string("img_root_lkm_patch"))
        
        check_init_cmd = [str(magiskboot_exe), "cpio", "ramdisk.cpio", "exists init"]
        init_exists_proc = utils.run_command(check_init_cmd, cwd=work_dir, check=False, capture=True)
        
        if init_exists_proc.returncode == 0:
            print(get_string("img_root_lkm_backup_init"))
            utils.run_command([str(magiskboot_exe), "cpio", "ramdisk.cpio", "mv init init.real"], cwd=work_dir)

        print(get_string("img_root_lkm_add_files"))
        utils.run_command([str(magiskboot_exe), "cpio", "ramdisk.cpio", "add 0755 init init"], cwd=work_dir)
        utils.run_command([str(magiskboot_exe), "cpio", "ramdisk.cpio", "add 0755 kernelsu.ko kernelsu.ko"], cwd=work_dir)
        
        print(get_string("img_root_step6_init_boot").format(name=img_name))
        utils.run_command([str(magiskboot_exe), "repack", img_name], cwd=work_dir)
        if not (work_dir / "new-boot.img").exists():
            print(get_string("img_root_repack_fail"))
            return None
        shutil.move(work_dir / "new-boot.img", patched_boot_path)
        print(get_string("img_root_repack_ok"))

        return patched_boot_path

def get_kernel_version(file_path: Union[str, Path]) -> Optional[str]:
    kernel_file = Path(file_path)
    if not kernel_file.exists():
        print(get_string("img_kv_err_not_found").format(path=file_path), file=sys.stderr)
        return None

    try:
        content = kernel_file.read_bytes()
        potential_strings = re.findall(b'[ -~]{10,}', content)
        
        found_version = None
        for string_bytes in potential_strings:
            try:
                line = string_bytes.decode('ascii', errors='ignore')
                if 'Linux version ' in line:
                    base_version_match = re.search(r'(\d+\.\d+\.\d+)', line)
                    if base_version_match:
                        found_version = base_version_match.group(1)
                        print(get_string("img_kv_found").format(line=line.strip()), file=sys.stderr)
                        break
            except UnicodeDecodeError:
                continue

        if found_version:
            return found_version
        else:
            print(get_string("img_kv_err_parse"), file=sys.stderr)
            return None

    except Exception as e:
        print(get_string("unexpected_error").format(e=e), file=sys.stderr)
        return None