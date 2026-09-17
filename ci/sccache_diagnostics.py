"""Keep raw server logs private; publish only allowlisted diagnostic values."""

import argparse
from collections import Counter
import json
import os
from pathlib import Path
import re
import subprocess


COUNTERS = ("cache_writes", "cache_write_errors", "cache_read_errors", "cache_timeouts")
KINDS = ("RateLimited", "PermissionDenied", "AlreadyExists", "NotFound",
         "Unexpected", "ConfigInvalid", "Unsupported")


def classify(lines):
    """Count fixed categories, never copy error text, URLs, paths, or headers."""
    statuses, kinds = Counter(), Counter()
    errors = 0
    for line in lines:
        if "Error executing cache write:" not in line:
            continue
        errors += 1
        # OpenDAL formats response status in its context / Response Debug output.
        codes = set(re.findall(r'\b(?:status|status_code|http_status)\b["\s:=]+([45]\d{2})\b',
                               line, flags=re.I))
        codes.update(re.findall(r'\bHTTP(?:/\d(?:\.\d)?)?\s+([45]\d{2})\b', line))
        statuses.update(codes or {"unknown"})
        found = {kind for kind in KINDS if re.search(rf"\b{kind}\b", line)}
        kinds.update(found or {"unknown"})
    return {"write_error_log_entries": errors, "http_statuses": dict(statuses),
            "error_kinds": dict(kinds)}


def configure(directory):
    directory.mkdir(parents=True, exist_ok=True)
    with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as output:
        output.write(f"SCCACHE_ERROR_LOG={directory / 'server.log'}\n")
        # This is the module that logs the underlying cache write error at debug.
        # Avoid enabling HTTP transport tracing (which can log credentials).
        output.write("SCCACHE_LOG=off,sccache::server=debug\n")


def report(directory):
    directory.mkdir(parents=True, exist_ok=True)
    log = directory / "server.log"
    data = {"sccache_version": "0.18.0", "log_present": log.exists(),
            "stats_status": "unavailable", "counters": {}}
    if log.exists():
        with log.open(encoding="utf-8", errors="replace") as source:
            data.update(classify(source))
        try:
            result = subprocess.run(["sccache", "--show-stats", "--stats-format=json"],
                                    capture_output=True, text=True, timeout=30, check=False)
            if result.returncode == 0:
                stats = json.loads(result.stdout)["stats"]
                data["counters"] = {key: stats[key] for key in COUNTERS
                                    if type(stats.get(key)) is int}
                data["stats_status"] = "collected"
        except (OSError, subprocess.TimeoutExpired, ValueError, KeyError):
            # Do not print subprocess output or exception text: it may contain secrets.
            pass
    (directory / "report.json").write_text(json.dumps(data, indent=2), encoding="utf-8")
    text = "\n### sccache diagnostics (0.18.0)\n\n"
    text += f"Log present: {data['log_present']}; statistics: {data['stats_status']}.\n\n"
    text += "| Counter | Count |\n|---|---:|\n"
    for key, value in data["counters"].items():
        text += f"| {key} | {value} |\n"
    for group in ("http_statuses", "error_kinds"):
        for key, value in data.get(group, {}).items():
            text += f"| {group}: {key} | {value} |\n"
    text += "\nCounts describe log entries, not necessarily unique requests. "
    text += "Unknown means the error did not contain a recognized code/category. "
    text += "Raw server logs are not uploaded.\n"
    if summary := os.getenv("GITHUB_STEP_SUMMARY"):
        with open(summary, "a", encoding="utf-8") as output:
            output.write(text)
    print(text)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["configure", "report"])
    args = parser.parse_args()
    directory = Path(os.environ["RUNNER_TEMP"]) / "sccache-diagnostics"
    (configure if args.operation == "configure" else report)(directory)
