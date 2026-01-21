import os
import shutil
import sys
from pathlib import Path
from unittest.mock import patch

import cache_fw
import py7zr
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
        cache_fw.ensure_archive_downloaded()
    except Exception as e:
        if cache_fw.ARCHIVE.exists():
            cache_fw.ARCHIVE.unlink()
        pytest.fail(f"Download failed: {e}")
    cached_url = cache_fw.read_cached_url()
    cache_fw.EXTRACT_DIR.mkdir(parents=True, exist_ok=True)
    targets = [
        "vbmeta.img",
        "boot.img",
        "vendor_boot.img",
        "rawprogram_unsparse0.xml",
        "rawprogram_save_persist_unsparse0.xml",
    ]

    cached_map = {}
    missing_targets = False
    for t in targets:
        found = list(cache_fw.EXTRACT_DIR.rglob(t))
        if found:
            cached_map[t] = found[0]
        else:
            missing_targets = True
            break

    if not missing_targets and cached_url == cache_fw.FW_URL:
        print("\n[INFO] Using cached extracted files.", flush=True)
        return cached_map

    print("\n[INFO] Extracting archive...", flush=True)
    try:
        if cache_fw.EXTRACT_DIR.exists():
            shutil.rmtree(cache_fw.EXTRACT_DIR)
        cache_fw.EXTRACT_DIR.mkdir(parents=True, exist_ok=True)

        with py7zr.SevenZipFile(
            cache_fw.ARCHIVE, mode="r", password=cache_fw.FW_PW
        ) as z:
            all_f = z.getnames()
            to_ext = [
                f
                for f in all_f
                if os.path.basename(f.replace("\\", "/")) in targets
                and "/image/" in f.replace("\\", "/")
            ]

            if not to_ext:
                pytest.fail("Targets not found in archive")
            z.extract(path=cache_fw.EXTRACT_DIR, targets=to_ext)

            mapped = {}
            for t in targets:
                for p in cache_fw.EXTRACT_DIR.rglob(t):
                    mapped[t] = p
                    break
            return mapped

    except Exception as e:
        pytest.fail(f"Extraction failed: {e}")


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
