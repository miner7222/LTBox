import os
import sys
from pathlib import Path
from unittest.mock import patch

import cache_fw
import pytest
from ltbox import downloader, i18n

sys.path.append(os.path.abspath(os.path.join(os.path.dirname(__file__), "../bin")))
sys.path.append(str(Path(__file__).parent))


@pytest.fixture(scope="session", autouse=True)
def setup_language():
    i18n.load_lang("en")


@pytest.fixture(scope="session", autouse=True)
def setup_external_tools():
    print("\n[INFO] Setting up external tools for integration tests...", flush=True)
    try:
        downloader.ensure_avb_tools()
    except Exception as e:
        print(f"\n[WARN] Failed to setup tools: {e}", flush=True)


@pytest.fixture(autouse=True)
def mock_python_executable():
    with patch("ltbox.constants.PYTHON_EXE", sys.executable):
        yield


@pytest.fixture(scope="module")
def fw_pkg(tmp_path_factory):
    if not cache_fw.FW_PW:
        pytest.skip("TEST_FW_PASSWORD not set")

    try:
        cache_fw.ensure_firmware_extracted()
    except Exception as e:
        pytest.fail(f"Firmware preparation failed: {e}")

    cached_map = {}
    missing_targets = False

    for t in cache_fw.TARGETS:
        found = list(cache_fw.EXTRACT_DIR.rglob(t))
        if found:
            cached_map[t] = found[0]
        else:
            missing_targets = True
            break

    if missing_targets:
        pytest.fail("Targets missing despite successful cache preparation")

    print("\n[INFO] Using cached extracted files.", flush=True)
    return cached_map


@pytest.fixture
def mock_env(tmp_path):
    dirs = {
        "IMAGE_DIR": tmp_path / "image",
        "OUTPUT_DP_DIR": tmp_path / "output_dp",
        "OUTPUT_DIR": tmp_path / "output",
        "OUTPUT_ANTI_ROLLBACK_DIR": tmp_path / "output_arb",
        "OUTPUT_XML_DIR": tmp_path / "output_xml",
        "EDL_LOADER_FILE": tmp_path / "loader.elf",
    }
    for d in dirs.values():
        if d.suffix:
            d.parent.mkdir(parents=True, exist_ok=True)
            d.touch()
        else:
            d.mkdir(parents=True, exist_ok=True)

    with patch.multiple("ltbox.constants", **dirs):
        yield dirs
