"""Record cache experiment timings while preserving the measured command's status."""

import argparse
import json
import os
import shutil
from pathlib import Path
import subprocess
import time


def record(directory: Path, name: str, **values) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    (directory / f"{name}.json").write_text(json.dumps(values, indent=2), encoding="utf-8")


def finish(directory: Path, name: str, **values) -> None:
    path = directory / f"{name}.json"
    start = json.loads(path.read_text(encoding="utf-8"))["started"]
    record(directory, name, seconds=round(time.time() - start, 3), **values)


def measure(directory: Path, name: str, command: list[str]) -> int:
    record(directory, name, started=time.time())
    result = subprocess.run(command, check=False)
    finish(directory, name, exit_code=result.returncode)
    return result.returncode


def report(directory: Path, target: Path, cache_hit: str) -> None:
    rows = {path.stem: json.loads(path.read_text(encoding="utf-8"))
            for path in directory.glob("*.json")
            if path.stem in {"dependency-cache-restore", "workspace-tests", "demo-tests"}}
    target_bytes = sum(path.stat().st_size for path in target.rglob("*") if path.is_file())
    data = {"commit": os.getenv("GITHUB_SHA"), "run": os.getenv("GITHUB_RUN_ID"),
            "target_cache_exact_hit": cache_hit == "true", "phases": rows,
            "target_bytes_before_cache_pruning": target_bytes}
    record(directory, "summary", **data)
    if os.getenv("RUSTC_WRAPPER") and shutil.which("sccache"):
        stats = subprocess.run(["sccache", "--show-stats", "--stats-format=json"],
                               capture_output=True, text=True, timeout=30, check=False)
        (directory / "sccache.json").write_text(stats.stdout, encoding="utf-8")
    else:
        record(directory, "sccache", enabled=False)
    summary = os.getenv("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as output:
            output.write("\n### Windows dependency cache experiment\n\n")
            output.write(f"Exact cache hit: **{cache_hit == 'true'}**\n\n")
            output.write("| Phase | Seconds |\n|---|---:|\n")
            for name, row in rows.items():
                output.write(f"| {name} | {row.get('seconds', 'incomplete')} |\n")
            output.write(f"\nTarget before pruning: {target_bytes / 2**30:.2f} GiB. "
                         "This is not the compressed cache size. See post-job cache logs "
                         "for archive size and save time; sccache statistics are attached.\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["start", "finish", "run", "report"])
    parser.add_argument("name")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    directory = Path(os.environ.get("CI_METRICS_DIR")
                     or Path(os.environ["RUNNER_TEMP"]) / "cache-metrics")
    if args.operation == "start":
        record(directory, args.name, started=time.time())
    elif args.operation == "finish":
        finish(directory, args.name)
    elif args.operation == "run":
        raise SystemExit(measure(directory, args.name, args.command))
    else:
        report(directory, Path(args.name), os.getenv("TARGET_CACHE_HIT", "false"))
