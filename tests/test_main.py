import json
import os
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest
from ltbox import main

sys.path.append(os.path.abspath(os.path.join(os.path.dirname(__file__), "../bin")))


class TestApp:
    def test_imports(self):
        assert hasattr(main, "CommandRegistry")
        assert hasattr(main, "setup_console")

    def test_registry(self):
        reg = main.CommandRegistry()

        def dummy():
            return "ok"

        reg.add("cmd", dummy, "Title", require_dev=False)
        c = reg.get("cmd")
        assert c["title"] == "Title"
        assert c["func"]() == "ok"

    def test_json_validity(self):
        d = Path(__file__).parent.parent / "bin/ltbox"
        files = list(d.rglob("*.json"))
        if not files:
            pytest.skip("No JSON")

        for f in files:
            with open(f, "r", encoding="utf-8") as fp:
                json.load(fp)

    def test_config_keys(self):
        p = Path(__file__).parent.parent / "bin/ltbox/config.json"
        if p.exists():
            with open(p, "r", encoding="utf-8") as f:
                c = json.load(f)
            assert "version" in c


def test_main_loop_settings_flow(monkeypatch, tmp_path):
    settings_path = tmp_path / "settings.json"
    store = main.SettingsStore(settings_path)
    monkeypatch.setattr(main, "SETTINGS_STORE", store)

    actions = iter(
        [
            "menu_settings",
            "toggle_region",
            "toggle_adb",
            "toggle_rollback",
            "back",
            "exit",
        ]
    )

    def fake_select_menu_action(menu_items, title_key):
        return next(actions)

    monkeypatch.setattr(main, "select_menu_action", fake_select_menu_action)

    class DummyController:
        last_instance = None

        def __init__(self, skip_adb=False):
            self.skip_adb = skip_adb
            DummyController.last_instance = self

    main.main_loop(DummyController, main.CommandRegistry(), settings_store=store)

    assert DummyController.last_instance.skip_adb is True
    assert store.load().target_region == "ROW"


def test_run_info_scan_creates_log(tmp_path):
    image_dir = tmp_path / "images"
    image_dir.mkdir()
    (image_dir / "boot.img").write_bytes(b"fake")
    (image_dir / "vendor.img").write_bytes(b"fake")
    (image_dir / "ignore.txt").write_text("skip")

    extra_img = tmp_path / "extra.img"
    extra_img.write_bytes(b"fake")

    calls = []

    def fake_run_command(cmd, capture=True, check=False):
        calls.append(cmd)
        return SimpleNamespace(stdout="FAKE-INFO", stderr="")

    constants = SimpleNamespace(
        BASE_DIR=tmp_path, PYTHON_EXE=Path("python"), AVBTOOL_PY=Path("avbtool.py")
    )
    avb_patch = SimpleNamespace(utils=SimpleNamespace(run_command=fake_run_command))

    main.run_info_scan([str(image_dir), str(extra_img)], constants, avb_patch)

    assert len(calls) == 3
    logs = list(tmp_path.glob("image_info_*.txt"))
    assert len(logs) == 1
    assert "FAKE-INFO" in logs[0].read_text(encoding="utf-8")
