import sys
import os
import json
import pytest
from pathlib import Path
from unittest.mock import patch

sys.path.append(os.path.abspath(os.path.join(os.path.dirname(__file__), '../bin')))

from ltbox import main

class TestAppStructure:

    def test_smoke_imports(self):
        assert hasattr(main, "CommandRegistry"), "CommandRegistry missing in main"
        assert hasattr(main, "setup_console"), "setup_console missing in main"

    def test_command_registry(self):
        registry = main.CommandRegistry()

        def dummy_func(): return "ok"

        registry.add("test_cmd", dummy_func, "Test Title", require_dev=False)
        cmd = registry.get("test_cmd")

        assert cmd is not None
        assert cmd["title"] == "Test Title"
        assert cmd["func"]() == "ok"

    def test_json_configs_validity(self):
        base_dir = Path(__file__).parent.parent / "bin/ltbox"
        json_files = list(base_dir.rglob("*.json"))

        assert len(json_files) > 0, "No JSON config files found to test"

        for json_file in json_files:
            with open(json_file, "r", encoding="utf-8") as f:
                data = json.load(f)
                assert isinstance(data, (dict, list)), f"Invalid JSON structure in {json_file.name}"

    def test_config_required_keys(self):
        config_path = Path(__file__).parent.parent / "bin/ltbox/config.json"
        if config_path.exists():
            with open(config_path, "r", encoding="utf-8") as f:
                config = json.load(f)
            assert "version" in config
            assert "magiskboot" in config

    @patch("platform.system", return_value="Linux")
    def test_platform_check_enforcement(self, mock_platform):
        with pytest.raises(SystemExit):
            main._check_platform()
