import sys
import os
import re
import json
import pytest
from pathlib import Path
from typing import Set, Dict

BASE_DIR = Path(__file__).parent.parent
SRC_DIR = BASE_DIR / "bin" / "ltbox"
LANG_DIR = SRC_DIR / "lang"

def get_source_keys() -> Set[str]:
    keys = set()
    pattern = re.compile(r'get_string\s*\(\s*["\']([^"\']+)["\']\s*\)')

    for py_file in SRC_DIR.rglob("*.py"):
        try:
            content = py_file.read_text(encoding="utf-8")
            matches = pattern.findall(content)
            keys.update(matches)
        except Exception as e:
            print(f"Warning: Failed to read {py_file}: {e}")

    return keys

def load_lang_files() -> Dict[str, Set[str]]:
    lang_keys = {}
    if not LANG_DIR.exists():
        return {}

    for json_file in LANG_DIR.glob("*.json"):
        try:
            with open(json_file, "r", encoding="utf-8") as f:
                data = json.load(f)
                if isinstance(data, dict):
                    lang_keys[json_file.name] = set(data.keys())
        except Exception as e:
            pytest.fail(f"Failed to load language file {json_file.name}: {e}")

    return lang_keys

class TestI18nIntegrity:

    @pytest.fixture(scope="class")
    def source_keys(self):
        return get_source_keys()

    @pytest.fixture(scope="class")
    def lang_map(self):
        return load_lang_files()

    def test_keys_exist_in_lang_files(self, source_keys, lang_map):
        if not lang_map:
            pytest.skip("No language files found in bin/ltbox/lang/")

        for filename, file_keys in lang_map.items():
            missing_keys = source_keys - file_keys

            assert not missing_keys, f"Found keys used in code but missing in {filename}: {missing_keys}"

    def test_parity_between_languages(self, lang_map):
        if not lang_map:
            pytest.skip("No language files found")

        base_lang = "en.json"
        if base_lang not in lang_map:
            base_lang = list(lang_map.keys())[0]

        base_keys = lang_map[base_lang]

        for filename, file_keys in lang_map.items():
            if filename == base_lang:
                continue

            missing_in_target = base_keys - file_keys
            assert not missing_in_target, f"Keys present in {base_lang} but missing in {filename}: {missing_in_target}"

            extra_in_target = file_keys - base_keys
            if extra_in_target:
                pytest.fail(f"Keys present in {filename} but missing in {base_lang} (deprecated?): {extra_in_target}")
