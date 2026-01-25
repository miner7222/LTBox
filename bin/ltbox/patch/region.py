from pathlib import Path
from typing import Any, Dict, Optional, Tuple

from .. import constants as const
from .. import utils
from ..i18n import get_string

EU_COUNTRY_CODES = {
    "AT",
    "BE",
    "BG",
    "HR",
    "CY",
    "CZ",
    "DK",
    "EE",
    "FI",
    "FR",
    "DE",
    "GR",
    "HU",
    "IE",
    "IT",
    "LV",
    "LT",
    "LU",
    "MT",
    "NL",
    "PL",
    "PT",
    "RO",
    "SK",
    "SI",
    "ES",
    "SE",
}


def _country_suffix(code: str) -> str:
    return "XE" if code.upper() in EU_COUNTRY_CODES else "XX"


def _candidate_suffixes(code: str) -> Tuple[str, ...]:
    if code.upper() in EU_COUNTRY_CODES:
        return ("XE", "XX")
    return ("XX",)


def _patch_vendor_boot_logic(
    content: bytes, **kwargs: Any
) -> Tuple[bytes, Dict[str, Any]]:
    target_region = str(kwargs.get("target_region", "PRC")).upper()
    if target_region == "ROW":
        patterns_map = {
            const.PRC_PATTERN_DOT: const.ROW_PATTERN_DOT,
            const.PRC_PATTERN_I: const.ROW_PATTERN_I,
        }
        target_patterns = [const.ROW_PATTERN_DOT, const.ROW_PATTERN_I]
    else:
        target_region = "PRC"
        patterns_map = {
            const.ROW_PATTERN_DOT: const.PRC_PATTERN_DOT,
            const.ROW_PATTERN_I: const.PRC_PATTERN_I,
        }
        target_patterns = [const.PRC_PATTERN_DOT, const.PRC_PATTERN_I]

    modified_content = content
    found_row_count = 0

    for target, replacement in patterns_map.items():
        count = content.count(target)
        if count > 0:
            utils.ui.info(
                get_string("img_vb_found_replace").format(
                    pattern=target.hex().upper(), count=count
                )
            )
            modified_content = modified_content.replace(target, replacement)
            found_row_count += count

    if found_row_count > 0:
        return modified_content, {
            "changed": True,
            "message": get_string("img_code_replaced_total").format(
                count=found_row_count
            ),
        }

    found_target = any(content.count(target) > 0 for target in target_patterns)
    if found_target:
        return content, {
            "changed": False,
            "message": get_string("img_vb_already_target").format(target=target_region),
        }

    return content, {"changed": False, "message": get_string("img_vb_no_patterns")}


def edit_vendor_boot(
    input_file_path: str, copy_if_unchanged: bool = True, target_region: str = "PRC"
) -> bool:
    input_file = Path(input_file_path)
    output_file = input_file.parent / "vendor_boot_prc.img"

    success = utils._process_binary_file(
        input_file,
        output_file,
        _patch_vendor_boot_logic,
        copy_if_unchanged=copy_if_unchanged,
        target_region=target_region,
    )

    if copy_if_unchanged and not success:
        raise RuntimeError(get_string("err_process_vendor_boot"))

    return success


def detect_country_codes() -> Dict[str, Optional[str]]:
    results: Dict[str, Optional[str]] = {}
    files_to_check = ["devinfo.img", "persist.img"]

    if not const.COUNTRY_CODES:
        utils.ui.error(get_string("img_det_warn_empty"))
        return {f: None for f in files_to_check}

    for filename in files_to_check:
        file_path = const.BASE_DIR / filename
        results[filename] = None

        if not file_path.exists():
            continue

        try:
            content = file_path.read_bytes()
            for code, _ in const.COUNTRY_CODES.items():
                for suffix in _candidate_suffixes(code):
                    target_bytes = f"{code.upper()}{suffix}".encode("ascii")
                    if target_bytes in content:
                        results[filename] = code
                        break
                if results[filename]:
                    break
        except Exception as e:
            utils.ui.error(get_string("img_det_err_read").format(name=filename, e=e))

    return results


def _patch_country_code_logic(
    content: bytes, **kwargs: Any
) -> Tuple[bytes, Dict[str, Any]]:
    current_code = kwargs.get("current_code")
    replacement_code = kwargs.get("replacement_code")

    if not current_code or not replacement_code:
        return content, {"changed": False, "message": get_string("img_code_invalid")}

    replacement_suffix = _country_suffix(replacement_code)
    replacement_string = f"{replacement_code.upper()}{replacement_suffix}"
    replacement_bytes = replacement_string.encode("ascii")

    target_strings = [
        f"{current_code.upper()}{suffix}"
        for suffix in _candidate_suffixes(current_code)
    ]
    target_bytes_list = [target.encode("ascii") for target in target_strings]
    targets_to_replace = [
        target for target in target_bytes_list if target != replacement_bytes
    ]

    if not targets_to_replace:
        return content, {
            "changed": False,
            "message": get_string("img_code_already").format(
                code=replacement_code.upper()
            ),
        }

    count = sum(content.count(target) for target in targets_to_replace)
    if count > 0:
        utils.ui.info(
            get_string("img_code_replace").format(
                target=", ".join(target_strings),
                count=count,
                replacement=replacement_string,
            )
        )
        modified_content = content
        for target_bytes in targets_to_replace:
            modified_content = modified_content.replace(target_bytes, replacement_bytes)
        return modified_content, {
            "changed": True,
            "message": get_string("img_code_replaced_total").format(count=count),
            "count": count,
        }

    return content, {
        "changed": False,
        "message": get_string("img_code_not_found").format(
            target=", ".join(target_strings)
        ),
    }


def patch_country_codes(
    replacement_code: str, target_map: Dict[str, Optional[str]]
) -> int:
    if not replacement_code or len(replacement_code) != 2:
        msg = get_string("img_patch_code_err").format(code=replacement_code)
        utils.ui.error(msg)
        raise RuntimeError(msg)

    total_patched = 0
    files_to_output = {
        "devinfo.img": "devinfo_modified.img",
        "persist.img": "persist_modified.img",
    }

    utils.ui.info(get_string("img_patch_start").format(code=replacement_code))

    for filename, current_code in target_map.items():
        if filename not in files_to_output:
            continue

        input_file = const.BASE_DIR / filename
        output_file = const.BASE_DIR / files_to_output[filename]

        if not input_file.exists():
            continue

        utils.ui.info(get_string("img_patch_processing").format(name=input_file.name))

        if not current_code:
            utils.ui.info(get_string("img_patch_skip").format(name=filename))
            continue

        success = utils._process_binary_file(
            input_file,
            output_file,
            _patch_country_code_logic,
            copy_if_unchanged=True,
            current_code=current_code,
            replacement_code=replacement_code,
        )

        if success:
            total_patched += 1

    utils.ui.info(get_string("img_patch_finish"))
    return total_patched
