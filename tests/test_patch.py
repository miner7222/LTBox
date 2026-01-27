import os
import shutil
import sys
from unittest.mock import patch

import pytest
from ltbox.patch import avb

sys.path.append(os.path.abspath(os.path.join(os.path.dirname(__file__), "../bin")))

pytestmark = pytest.mark.integration


def test_vbmeta_parse(fw_pkg):
    path = fw_pkg.get("vbmeta.img")
    assert path and path.exists()

    info = avb.extract_image_avb_info(path)
    assert info["algorithm"] == "SHA256_RSA4096"


def test_boot_parse(fw_pkg):
    path = fw_pkg.get("boot.img")
    assert path and path.exists()

    info = avb.extract_image_avb_info(path)
    assert int(info["partition_size"]) > int(info["data_size"])


def test_process_boot_image_avb_reapplies_footer(fw_pkg, tmp_path):
    boot_img = fw_pkg.get("boot.img")
    if not boot_img:
        pytest.skip("boot.img not found in firmware package")

    boot_bak = tmp_path / "boot.bak.img"
    target_boot = tmp_path / "boot_target.img"
    shutil.copy(boot_img, boot_bak)
    shutil.copy(boot_img, target_boot)

    boot_info = avb.extract_image_avb_info(boot_bak)

    with patch("ltbox.constants.BASE_DIR", tmp_path):
        avb.process_boot_image_avb(target_boot, gki=True)

    patched_info = avb.extract_image_avb_info(target_boot)

    for key in ["algorithm", "name", "rollback", "salt"]:
        assert patched_info.get(key) == boot_info.get(key)

    assert int(patched_info["partition_size"]) >= int(
        patched_info.get("data_size", patched_info["partition_size"])
    )


def test_patch_chained_image_rollback_noop(fw_pkg, tmp_path):
    init_boot = fw_pkg.get("init_boot.img")
    if not init_boot:
        pytest.skip("init_boot.img not found in firmware package")

    source = tmp_path / "init_boot.img"
    patched = tmp_path / "init_boot_patched.img"
    shutil.copy(init_boot, source)

    info = avb.extract_image_avb_info(source)
    current_rb = int(info.get("rollback", "0"))

    avb.patch_chained_image_rollback("init_boot", current_rb, source, patched)

    assert patched.exists()
    assert patched.read_bytes() == source.read_bytes()
