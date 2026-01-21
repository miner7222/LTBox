from __future__ import annotations

import math
import os
import shutil
import threading
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Callable, Dict, Optional

import requests

FW_URL = (
    "http://zsk-cdn.lenovows.com/%E7%9F%A5%E8%AF%86%E5%BA%93/"
    "Flash_tool_image/TB322_ZUXOS_1.5.10.063_Tool.7z"
)
FW_PW = os.environ.get("TEST_FW_PASSWORD")

CACHE_DIR = Path(__file__).parent / "data"
ARCHIVE = CACHE_DIR / "fw_archive.7z"
EXTRACT_DIR = CACHE_DIR / "extracted"
URL_RECORD_FILE = CACHE_DIR / "url.txt"
PART_SUFFIX = ".part"
DEFAULT_SEGMENTS = 4
DOWNLOAD_TIMEOUT = 30
DOWNLOAD_CHUNK_SIZE = 1024 * 1024


def read_cached_url() -> str:
    if not URL_RECORD_FILE.exists():
        return ""
    try:
        return URL_RECORD_FILE.read_text("utf-8").strip()
    except Exception:
        return ""


def reset_cache_if_url_changed(cached_url: str) -> None:
    if cached_url != FW_URL:
        print("\n[INFO] URL Changed or Cache missing. Cleaning up...", flush=True)
        if ARCHIVE.exists():
            ARCHIVE.unlink()
        if EXTRACT_DIR.exists():
            shutil.rmtree(EXTRACT_DIR)
        if URL_RECORD_FILE.exists():
            URL_RECORD_FILE.unlink()


def _download_stream(
    url: str,
    dest_path: Path,
    headers: Optional[Dict[str, str]] = None,
    timeout: int = DOWNLOAD_TIMEOUT,
    on_progress: Optional[Callable[[int], None]] = None,
) -> None:
    with requests.get(url, stream=True, headers=headers, timeout=timeout) as response:
        response.raise_for_status()
        with open(dest_path, "wb") as f:
            for chunk in response.iter_content(chunk_size=DOWNLOAD_CHUNK_SIZE):
                if chunk:
                    f.write(chunk)
                    if on_progress:
                        on_progress(len(chunk))


def _download_range(
    url: str,
    start: int,
    end: int,
    dest_path: Path,
    headers: Optional[Dict[str, str]] = None,
    timeout: int = DOWNLOAD_TIMEOUT,
    on_progress: Optional[Callable[[int], None]] = None,
) -> None:
    range_headers = {"Range": f"bytes={start}-{end}"}
    if headers:
        range_headers.update(headers)

    with requests.get(
        url, stream=True, headers=range_headers, timeout=timeout
    ) as response:
        response.raise_for_status()
        if response.status_code not in (200, 206):
            raise RuntimeError(f"Unexpected status code: {response.status_code}")
        with open(dest_path, "wb") as f:
            for chunk in response.iter_content(chunk_size=DOWNLOAD_CHUNK_SIZE):
                if chunk:
                    f.write(chunk)
                    if on_progress:
                        on_progress(len(chunk))


def _render_progress(downloaded: int, total_size: int, start_time: float) -> str:
    elapsed = max(time.monotonic() - start_time, 1e-6)
    speed = downloaded / elapsed
    eta_seconds = (total_size - downloaded) / speed if speed > 0 else 0
    percent = (downloaded / total_size) * 100 if total_size else 0
    return (
        f"\rDownloading... {percent:6.2f}% "
        f"({downloaded / (1024**2):.2f} MB / {total_size / (1024**2):.2f} MB) "
        f"Speed: {speed / (1024**2):.2f} MB/s "
        f"ETA: {eta_seconds:,.0f}s"
    )


def download_with_ranges(
    url: str,
    dest_path: Path,
    segments: int = DEFAULT_SEGMENTS,
    headers: Optional[Dict[str, str]] = None,
    timeout: int = DOWNLOAD_TIMEOUT,
) -> None:
    head = requests.head(url, headers=headers, timeout=timeout, allow_redirects=True)
    head.raise_for_status()
    total_size = int(head.headers.get("Content-Length", "0"))
    accept_ranges = head.headers.get("Accept-Ranges", "").lower() == "bytes"
    downloaded = 0
    download_lock = threading.Lock()
    start_time = time.monotonic()
    stop_event = threading.Event()

    def report_progress() -> None:
        last_output = ""
        while not stop_event.is_set():
            with download_lock:
                current = downloaded
            if total_size:
                output = _render_progress(current, total_size, start_time)
                if output != last_output:
                    print(output, end="", flush=True)
                    last_output = output
            time.sleep(0.5)

    def on_progress(bytes_count: int) -> None:
        nonlocal downloaded
        with download_lock:
            downloaded += bytes_count

    if total_size <= 0 or not accept_ranges or segments <= 1:
        reporter = threading.Thread(target=report_progress, daemon=True)
        reporter.start()
        try:
            _download_stream(
                url,
                dest_path,
                headers=headers,
                timeout=timeout,
                on_progress=on_progress,
            )
        finally:
            stop_event.set()
            reporter.join()
            if total_size:
                print(_render_progress(downloaded, total_size, start_time), flush=True)
        return

    part_paths = []
    part_size = math.ceil(total_size / segments)

    try:
        reporter = threading.Thread(target=report_progress, daemon=True)
        reporter.start()
        with ThreadPoolExecutor(max_workers=segments) as executor:
            futures = []
            for idx in range(segments):
                start = idx * part_size
                end = min(start + part_size - 1, total_size - 1)
                part_path = dest_path.with_suffix(
                    f"{dest_path.suffix}{PART_SUFFIX}{idx}"
                )
                part_paths.append(part_path)
                futures.append(
                    executor.submit(
                        _download_range,
                        url,
                        start,
                        end,
                        part_path,
                        headers,
                        timeout,
                        on_progress,
                    )
                )

            for future in as_completed(futures):
                future.result()

        with open(dest_path, "wb") as output:
            for part_path in part_paths:
                with open(part_path, "rb") as part:
                    shutil.copyfileobj(part, output)

    except Exception:
        if dest_path.exists():
            dest_path.unlink()
        raise
    finally:
        stop_event.set()
        reporter.join()
        if total_size:
            print(_render_progress(downloaded, total_size, start_time), flush=True)
        for part_path in part_paths:
            if part_path.exists():
                part_path.unlink()


def ensure_archive_downloaded() -> None:
    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    cached_url = read_cached_url()
    reset_cache_if_url_changed(cached_url)

    if ARCHIVE.exists() and ARCHIVE.stat().st_size > 0:
        return

    print("\n[INFO] Starting download...", flush=True)
    headers = {
        "User-Agent": (
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
            "AppleWebKit/537.36 (KHTML, like Gecko) "
            "Chrome/120.0.0.0 Safari/537.36"
        )
    }

    download_with_ranges(
        FW_URL,
        ARCHIVE,
        segments=DEFAULT_SEGMENTS,
        headers=headers,
    )
    print(
        f"\n[INFO] Download Complete! Size: {ARCHIVE.stat().st_size / (1024**3):.2f} GB",
        flush=True,
    )
    URL_RECORD_FILE.write_text(FW_URL, encoding="utf-8")


def main() -> None:
    if not FW_PW:
        print("[INFO] TEST_FW_PASSWORD not set. Skipping FW cache prefetch.")
        return

    ensure_archive_downloaded()


if __name__ == "__main__":
    main()
