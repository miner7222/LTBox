import json
import os
import platform
import sys
from pathlib import Path

APP_DIR = Path(__file__).parent.resolve()
LANG_DIR = APP_DIR / "lang"

_lang_data = {}
_fallback_data = {}

def select_language() -> str:
    os.system('cls' if os.name == 'nt' else 'clear')
    if not LANG_DIR.is_dir():
        print(f"[!] Critical Error: Language directory not found.", file=sys.stderr)
        print(f"[!] Expected path: {LANG_DIR}", file=sys.stderr)
        if platform.system() == "Windows":
            os.system("pause")
        sys.exit(1)

    lang_files = sorted(list(LANG_DIR.glob("*.json")))
    if not lang_files:
        print(f"[!] Critical Error: No language files (*.json) found in:", file=sys.stderr)
        print(f"[!] Path: {LANG_DIR}", file=sys.stderr)
        if platform.system() == "Windows":
            os.system("pause")
        sys.exit(1)

    available_languages = {}
    menu_options = []
    
    for i, f in enumerate(lang_files, 1):
        lang_code = f.stem
        available_languages[str(i)] = lang_code
        
        try:
            with open(f, 'r', encoding='utf-8') as lang_file:
                temp_lang = json.load(lang_file)
                lang_name = temp_lang.get("lang_native_name", lang_code)
        except Exception:
            lang_name = lang_code
        menu_options.append(f"     {i}. {lang_name}")

    print("\n  " + "=" * 58)
    print("     Select Language")
    print("  " + "=" * 58 + "\n")
    print("\n".join(menu_options))
    print("\n  " + "=" * 58 + "\n")

    choice = ""
    while choice not in available_languages:
        prompt = f"    Enter your choice (1-{len(available_languages)}): "
        choice = input(prompt).strip()
        if choice not in available_languages:
            print(f"    [!] Invalid choice. Please enter a number from 1 to {len(available_languages)}.")
    
    return available_languages[choice]

def load_lang(lang_code: str = "en"):
    global _lang_data, _fallback_data
    
    fallback_file = LANG_DIR / "en.json"
    if not _fallback_data and fallback_file.exists():
        try:
            with open(fallback_file, 'r', encoding='utf-8') as f:
                _fallback_data = json.load(f)
        except Exception as e:
            print(f"[!] Failed to load fallback language en.json: {e}", file=sys.stderr)
            _fallback_data = {}

    if lang_code == "en" or not (LANG_DIR / f"{lang_code}.json").exists():
        _lang_data = _fallback_data
    else:
        lang_file = LANG_DIR / f"{lang_code}.json"
        try:
            with open(lang_file, 'r', encoding='utf-8') as f:
                _lang_data = json.load(f)
        except Exception as e:
            print(f"[!] Failed to load language {lang_code}, using fallback: {e}", file=sys.stderr)
            _lang_data = _fallback_data

def get_string(key: str, default: str = "") -> str:
    val = _lang_data.get(key, _fallback_data.get(key, default))
    if val:
        return val
    return f"[{key}]"