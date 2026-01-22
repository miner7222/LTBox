import re
import shutil
import sys
from pathlib import Path
from typing import Optional, Union

from .. import constants as const
from .. import device, downloader, utils
from ..i18n import get_string


def patch_boot_with_root_algo(
    work_dir: Path,
    magiskboot_exe: Path,
    dev: Optional[device.DeviceController] = None,
    gki: bool = False,
    lkm_kernel_version: Optional[str] = None,
    root_type: str = "ksu",
    skip_lkm_download: bool = False,
) -> Optional[Path]:

    img_name = const.FN_BOOT if gki else const.FN_INIT_BOOT
    out_img_name = const.FN_BOOT_ROOT if gki else const.FN_INIT_BOOT_ROOT

    patched_boot_path = const.BASE_DIR / out_img_name
    work_img_path = work_dir / img_name

    if not work_img_path.exists():
        print(
            get_string("img_root_err_img_not_found").format(name=img_name),
            file=sys.stderr,
        )
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
            print(
                get_string("img_root_kernel_invalid").format(ver=target_kernel_version)
            )
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

        return patched_boot_path

    else:
        print(get_string("img_root_step1_init_boot").format(name=img_name))
        utils.run_command([str(magiskboot_exe), "unpack", img_name], cwd=work_dir)
        if not (work_dir / "ramdisk.cpio").exists():
            print(get_string("img_root_unpack_fail"))
            return None
        print(get_string("img_root_unpack_ok"))

        if not skip_lkm_download:
            try:
                if root_type == "magisk":
                    print(get_string("img_root_magisk_download"))
                    apk_path = downloader.download_magisk_apk(work_dir)
                    downloader.extract_magisk_libs(apk_path, work_dir)
                else:
                    print(get_string("img_root_lkm_download"))
                    ksuinit_path = work_dir / "init"
                    kmod_path = work_dir / "kernelsu.ko"

                    if root_type == "sukisu":
                        if not lkm_kernel_version:
                            print(get_string("img_root_lkm_no_dev"), file=sys.stderr)
                            return None

                        downloader.download_nightly_artifacts(
                            repo=const.SUKISU_REPO,
                            workflow_id=const.SUKISU_WORKFLOW,
                            manager_name="Spoofed-Manager.zip",
                            mapped_name=lkm_kernel_version,
                            target_dir=work_dir,
                        )
                    else:
                        downloader.download_ksuinit_release(ksuinit_path)
                        if not lkm_kernel_version:
                            print(get_string("img_root_lkm_no_dev"), file=sys.stderr)
                            return None
                        downloader.get_lkm_kernel_release(kmod_path, lkm_kernel_version)

            except Exception as e:
                error_key = (
                    "img_root_magisk_download_fail"
                    if root_type == "magisk"
                    else "img_root_lkm_download_fail"
                )
                print(get_string(error_key).format(e=e), file=sys.stderr)
                return None
        else:
            print(get_string("img_root_skip_download"))

        if root_type == "magisk":
            print(get_string("img_root_magisk_patch"))
        else:
            print(get_string("img_root_lkm_patch"))

        check_init_cmd = [str(magiskboot_exe), "cpio", "ramdisk.cpio", "exists init"]
        init_exists_proc = utils.run_command(
            check_init_cmd, cwd=work_dir, check=False, capture=True
        )

        if init_exists_proc.returncode == 0 and root_type != "magisk":
            print(get_string("img_root_lkm_backup_init"))
            utils.run_command(
                [str(magiskboot_exe), "cpio", "ramdisk.cpio", "mv init init.real"],
                cwd=work_dir,
            )

        if root_type == "magisk":
            required_files = [
                "magiskinit",
                "magisk",
                "init-ld",
                "stub.apk",
            ]
            missing_files = [
                name for name in required_files if not (work_dir / name).exists()
            ]
            if missing_files:
                print(
                    get_string("img_root_magisk_missing").format(
                        files=", ".join(missing_files)
                    ),
                    file=sys.stderr,
                )
                return None

            print(get_string("img_root_magisk_add_files"))
            config_path = work_dir / "config"
            config_path.write_text(
                "\n".join(
                    [
                        "KEEPVERITY=false",
                        "KEEPFORCEENCRYPT=false",
                        "RECOVERYMODE=false",
                        "VENDORBOOT=false",
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            utils.run_command(
                [str(magiskboot_exe), "compress=xz", "magisk", "magisk.xz"],
                cwd=work_dir,
            )
            utils.run_command(
                [str(magiskboot_exe), "compress=xz", "stub.apk", "stub.xz"],
                cwd=work_dir,
            )
            utils.run_command(
                [str(magiskboot_exe), "compress=xz", "init-ld", "init-ld.xz"],
                cwd=work_dir,
            )
            utils.run_command(
                [
                    str(magiskboot_exe),
                    "cpio",
                    "ramdisk.cpio",
                    "add 0750 init magiskinit",
                    "mkdir 0750 overlay.d",
                    "mkdir 0750 overlay.d/sbin",
                    "add 0644 overlay.d/sbin/magisk.xz magisk.xz",
                    "add 0644 overlay.d/sbin/stub.xz stub.xz",
                    "add 0644 overlay.d/sbin/init-ld.xz init-ld.xz",
                    "patch",
                    "backup ramdisk.cpio.orig",
                    "mkdir 000 .backup",
                    "add 000 .backup/.magisk config",
                ],
                cwd=work_dir,
            )
            for temp_name in [
                "ramdisk.cpio.orig",
                "config",
                "magisk.xz",
                "stub.xz",
                "init-ld.xz",
            ]:
                temp_path = work_dir / temp_name
                if temp_path.exists():
                    temp_path.unlink()
        else:
            print(get_string("img_root_lkm_add_files"))
            utils.run_command(
                [str(magiskboot_exe), "cpio", "ramdisk.cpio", "add 0755 init init"],
                cwd=work_dir,
            )
            utils.run_command(
                [
                    str(magiskboot_exe),
                    "cpio",
                    "ramdisk.cpio",
                    "add 0755 kernelsu.ko kernelsu.ko",
                ],
                cwd=work_dir,
            )
        utils.run_command(
            [
                str(magiskboot_exe),
                "cpio",
                "ramdisk.cpio",
                "add 0755 kernelsu.ko kernelsu.ko",
            ],
            cwd=work_dir,
        )

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
        print(
            get_string("img_kv_err_not_found").format(path=file_path), file=sys.stderr
        )
        return None

    try:
        content = kernel_file.read_bytes()
        potential_strings = re.findall(b"[ -~]{10,}", content)

        found_version = None
        for string_bytes in potential_strings:
            try:
                line = string_bytes.decode("ascii", errors="ignore")
                if "Linux version " in line:
                    base_version_match = re.search(r"(\d+\.\d+\.\d+)", line)
                    if base_version_match:
                        found_version = base_version_match.group(1)
                        print(
                            get_string("img_kv_found").format(line=line.strip()),
                            file=sys.stderr,
                        )
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
