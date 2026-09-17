import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from sccache_diagnostics import classify, configure, report


class DiagnosticsTests(unittest.TestCase):
    def test_classification_does_not_copy_sensitive_error_context(self):
        result = classify([
            'DEBUG Error executing cache write: RateLimited (temporary) '
            'context: { status: 429 }, Response { status: 429, '
            'url: "https://cache.example/?sig=SECRET", Authorization: Bearer SECRET }',
            'DEBUG Error executing cache write: PermissionDenied status_code="403"',
            'DEBUG Error executing cache write: AlreadyExists HTTP 409',
            'DEBUG Error executing cache write: unknown failure SECRET',
            'DEBUG unrelated message status: 500 SECRET',
        ])
        self.assertEqual(result["http_statuses"], {"429": 1, "403": 1, "409": 1, "unknown": 1})
        self.assertEqual(result["write_error_log_entries"], 4)
        self.assertEqual(result["error_kinds"]["RateLimited"], 1)
        self.assertNotIn("SECRET", json.dumps(result))
        self.assertNotIn("cache.example", json.dumps(result))

    def test_missing_log_does_not_start_server(self):
        with tempfile.TemporaryDirectory() as temp, patch.dict(os.environ, {}, clear=True):
            directory = Path(temp)
            with patch("sccache_diagnostics.subprocess.run") as run:
                report(directory)
                run.assert_not_called()
            result = json.loads((directory / "report.json").read_text())
            self.assertFalse(result["log_present"])
            self.assertEqual(result["stats_status"], "unavailable")

    def test_report_allowlists_stats_and_configures_private_log(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            env = directory / "env"
            summary = directory / "summary"
            with patch.dict(os.environ, {"GITHUB_ENV": str(env), "GITHUB_STEP_SUMMARY": str(summary)}):
                configure(directory)
                self.assertIn("sccache::server=debug", env.read_text())
                (directory / "server.log").write_text("Error executing cache write: status: 429 SECRET\n")
                with patch("sccache_diagnostics.subprocess.run") as run:
                    run.return_value.returncode = 0
                    run.return_value.stdout = json.dumps({"stats": {
                        "cache_writes": 3, "cache_write_errors": 1,
                        "cache_read_errors": "SECRET", "private": "SECRET"}})
                    report(directory)
                result = (directory / "report.json").read_text()
                self.assertNotIn("SECRET", result + summary.read_text())
                self.assertEqual(json.loads(result)["counters"],
                                 {"cache_writes": 3, "cache_write_errors": 1})


if __name__ == "__main__":
    unittest.main()
