#!/usr/bin/env python3
"""Regenerate the bundled Noto Sans CJK subsets.

Why these files exist
---------------------
Upstream Noto Sans CJK ships one *family per weight* (the 500 weight declares
itself as "Noto Sans KR Medium", not as the Medium member of "Noto Sans KR"),
and the region subsets are ~4-8 MB each. Neither is usable directly:

* iced asks for `Font::with_name("Noto Sans KR")` at a given `Weight`. A face
  that names itself a different family never matches, so the weight request
  silently falls back to another typeface.
* Shipping full-coverage CJK faces costs ~17 MB for three weights we barely use.

So each face is subset to the glyphs the UI can actually render and its name
table is rewritten so all three weights belong to one family.

Run this only when `lang/*.json` gains characters outside the current subset, or
when upgrading the upstream Noto release.

    python3 -m venv .venv && .venv/bin/pip install fonttools brotli
    .venv/bin/python regenerate.py

Dynamic text (file paths, device names, log output) may contain characters
outside the subset; those fall back to system fonts, which is the same behaviour
as before and is why the subset only needs to cover the UI strings.
"""
import json
import pathlib
import subprocess
import sys
import urllib.request

from fontTools.ttLib import TTFont

HERE = pathlib.Path(__file__).parent
LANG_DIR = HERE.parent.parent / "lang"
UPSTREAM = "https://github.com/notofonts/noto-cjk/raw/main/Sans/SubsetOTF"
MONO_UPSTREAM = (
    "https://github.com/notofonts/notofonts.github.io/raw/main"
    "/fonts/NotoSansMono/hinted/ttf"
)
REGIONS = {"KR": "Noto Sans KR", "JP": "Noto Sans JP", "SC": "Noto Sans SC"}
WEIGHTS = {"Regular": 400, "Medium": 500, "Bold": 700}


def ui_glyphs() -> str:
    """Every character the localized UI strings can render, plus safe extras."""
    chars: set[str] = set()

    def walk(value):
        if isinstance(value, str):
            chars.update(value)
        elif isinstance(value, dict):
            for item in value.values():
                walk(item)
        elif isinstance(value, list):
            for item in value:
                walk(item)

    for path in sorted(LANG_DIR.glob("*.json")):
        walk(json.loads(path.read_text(encoding="utf-8")))

    chars |= set(latin_glyphs())
    return "".join(sorted(chars))


def latin_glyphs() -> str:
    """ASCII, Latin-1 and the punctuation the UI draws, in one place."""
    chars = {chr(c) for c in range(0x20, 0x7F)}        # ASCII
    chars |= {chr(c) for c in range(0xA0, 0x100)}     # Latin-1 supplement
    chars |= set("—–…“”‘’·•→←↑↓⇅✓×∙")                 # punctuation used in UI
    return "".join(sorted(chars))


def main() -> int:
    glyphs = HERE / "_glyphs.txt"
    glyphs.write_text(ui_glyphs(), encoding="utf-8")
    print(f"subset repertoire: {len(glyphs.read_text(encoding='utf-8'))} characters")

    for region, family in REGIONS.items():
        for weight, weight_class in WEIGHTS.items():
            src = HERE / f"_{region}-{weight}.otf"
            if not src.exists():
                url = f"{UPSTREAM}/{region}/NotoSans{region}-{weight}.otf"
                print(f"  downloading {url}")
                urllib.request.urlretrieve(url, src)

            dest = HERE / f"NotoSans{region}-{weight}.subset.otf"
            subprocess.run([
                sys.executable, "-m", "fontTools.subset", str(src),
                f"--text-file={glyphs}", f"--output-file={dest}",
                "--layout-features=*", "--no-hinting",
            ], check=True)

            # Rejoin the family: upstream puts 500 in its own "… Medium" family.
            font = TTFont(dest)
            full = family if weight == "Regular" else f"{family} {weight}"
            postscript = f"{family.replace(' ', '')}-{weight}"
            for record in list(font["name"].names):
                ids = {1: family, 2: weight, 4: full, 6: postscript,
                       16: family, 17: weight}
                if record.nameID in ids:
                    font["name"].setName(ids[record.nameID], record.nameID,
                                         record.platformID, record.platEncID,
                                         record.langID)
            font["OS/2"].usWeightClass = weight_class
            font.save(dest)
            print(f"  {dest.name}: {dest.stat().st_size // 1024} KB")

    # Fixed-width face for file paths and country codes. Latin only: a CJK
    # monospace runs to megabytes, and localized copy must not ask for this
    # face anyway. Upstream already names the family "Noto Sans Mono", so
    # unlike the CJK faces it needs no name-table repair.
    mono_src = HERE / "_Mono-Regular.ttf"
    if not mono_src.exists():
        url = f"{MONO_UPSTREAM}/NotoSansMono-Regular.ttf"
        print(f"  downloading {url}")
        urllib.request.urlretrieve(url, mono_src)
    mono_glyphs = HERE / "_mono_glyphs.txt"
    mono_glyphs.write_text(latin_glyphs(), encoding="utf-8")
    mono_dest = HERE / "NotoSansMono-Regular.subset.ttf"
    subprocess.run([
        sys.executable, "-m", "fontTools.subset", str(mono_src),
        f"--text-file={mono_glyphs}", f"--output-file={mono_dest}",
        "--layout-features=*", "--no-hinting",
    ], check=True)
    print(f"  {mono_dest.name}: {mono_dest.stat().st_size // 1024} KB")

    for tmp in list(HERE.glob("_*.otf")) + list(HERE.glob("_*.ttf")) + [
        glyphs,
        mono_glyphs,
    ]:
        tmp.unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
