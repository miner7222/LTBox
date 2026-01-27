import shutil
import xml.etree.ElementTree as ET
from unittest.mock import MagicMock, patch

import pytest
from ltbox import partition
from ltbox.actions import edl

pytestmark = pytest.mark.integration


def _copy_firmware_xml(fw_pkg, image_dir):
    candidates = [
        "rawprogram_unsparse0.xml",
        "rawprogram_save_persist_unsparse0.xml",
    ]
    for name in candidates:
        src = fw_pkg.get(name)
        if src:
            dest = image_dir / name
            shutil.copy(src, dest)
            return dest
    return None


def _get_first_program(xml_path):
    root = ET.parse(xml_path).getroot()
    program = next((p for p in root.findall("program") if p.get("label")), None)
    if program is None:
        pytest.skip("No program entries found in firmware XML")
    return program


def test_partition_params_from_firmware_xml(fw_pkg, mock_env):
    if not fw_pkg:
        pytest.skip("Firmware package not available")

    xml_path = _copy_firmware_xml(fw_pkg, mock_env["IMAGE_DIR"])
    if not xml_path:
        pytest.skip("Firmware XML not found")

    program = _get_first_program(xml_path)
    label = program.get("label")

    params = partition.ensure_params_or_fail(label)

    assert params["source_xml"] == xml_path.name
    assert params["lun"] == program.get("physical_partition_number")
    assert params["start_sector"] == program.get("start_sector")


def test_flash_partition_target_uses_firmware_params(fw_pkg, mock_env):
    if not fw_pkg:
        pytest.skip("Firmware package not available")

    xml_path = _copy_firmware_xml(fw_pkg, mock_env["IMAGE_DIR"])
    if not xml_path:
        pytest.skip("Firmware XML not found")

    program = _get_first_program(xml_path)
    label = program.get("label")

    image_path = mock_env["OUTPUT_DP_DIR"] / "patched.img"
    image_path.write_bytes(b"test")

    mock_dev = MagicMock()

    with patch("ltbox.actions.edl.utils.ui"):
        edl.flash_partition_target(mock_dev, "COM3", label, image_path)

    mock_dev.edl.write_partition.assert_called_once_with(
        port="COM3",
        image_path=image_path,
        lun=program.get("physical_partition_number"),
        start_sector=program.get("start_sector"),
    )
