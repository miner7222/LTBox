import os
import shutil
import sys
from pathlib import Path
from unittest.mock import patch

import py7zr
import pytest
from ltbox import downloader
from pypdl import Pypdl

sys.path.append(os.path.abspath(os.path.join(os.path.dirname(__file__), "../bin")))

QFIL_URL = "http://zsk-cdn.lenovows.com/%E7%9F%A5%E8%AF%86%E5%BA%93/Flash_tool_image/TB322_ZUXOS_1.5.10.063_Tool.7z"
QFIL_PW = os.environ.get("TEST_QFIL_PASSWORD")

CACHE_DIR = Path(__file__).parent / "data"
ARCHIVE = CACHE_DIR / "qfil_archive.7z"
EXTRACT_DIR = CACHE_DIR / "extracted"
URL_RECORD_FILE = CACHE_DIR / "url.txt"


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
    if not QFIL_PW:
        pytest.skip("TEST_QFIL_PASSWORD not set")

    CACHE_DIR.mkdir(parents=True, exist_ok=True)

    cached_url = ""
    if URL_RECORD_FILE.exists():
        try:
            cached_url = URL_RECORD_FILE.read_text("utf-8").strip()
        except Exception:
            pass

    if cached_url != QFIL_URL:
        print("\n[INFO] URL Changed or Cache missing. Cleaning up...", flush=True)
        if CACHE_DIR.exists():
            if ARCHIVE.exists():
                ARCHIVE.unlink()
            if EXTRACT_DIR.exists():
                shutil.rmtree(EXTRACT_DIR)
            if URL_RECORD_FILE.exists():
                URL_RECORD_FILE.unlink()

        CACHE_DIR.mkdir(parents=True, exist_ok=True)

    if not ARCHIVE.exists() or ARCHIVE.stat().st_size == 0:
        print("\n[INFO] Starting download...", flush=True)
        try:
            dl = Pypdl()
            dl.start(
                QFIL_URL,
                file_path=str(ARCHIVE),
                segments=10,
                display=True,
                block=True,
                retries=3,
            )
            print(
                f"\n[INFO] Download Complete! Size: {ARCHIVE.stat().st_size / (1024**3):.2f} GB",
                flush=True,
            )

            URL_RECORD_FILE.write_text(QFIL_URL, encoding="utf-8")

        except Exception as e:
            if ARCHIVE.exists():
                ARCHIVE.unlink()
            pytest.fail(f"Download failed: {e}")

    EXTRACT_DIR.mkdir(parents=True, exist_ok=True)
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
        found = list(EXTRACT_DIR.rglob(t))
        if found:
            cached_map[t] = found[0]
        else:
            missing_targets = True
            break

    if not missing_targets and cached_url == QFIL_URL:
        print("\n[INFO] Using cached extracted files.", flush=True)
        return cached_map

    print("\n[INFO] Extracting archive...", flush=True)
    try:
        if EXTRACT_DIR.exists():
            shutil.rmtree(EXTRACT_DIR)
        EXTRACT_DIR.mkdir(parents=True, exist_ok=True)

        with py7zr.SevenZipFile(ARCHIVE, mode="r", password=QFIL_PW) as z:
            all_f = z.getnames()
            to_ext = [
                f
                for f in all_f
                if os.path.basename(f.replace("\\", "/")) in targets
                and "/image/" in f.replace("\\", "/")
            ]

            if not to_ext:
                pytest.fail("Targets not found in archive")
            z.extract(path=EXTRACT_DIR, targets=to_ext)

            mapped = {}
            for t in targets:
                for p in EXTRACT_DIR.rglob(t):
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
