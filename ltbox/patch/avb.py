import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Dict, Optional, Any, List

from .. import constants as const
from .. import utils
from ..i18n import get_string

def extract_image_avb_info(image_path: Path) -> Dict[str, Any]:
    info_proc = utils.run_command(
        [str(const.PYTHON_EXE), str(const.AVBTOOL_PY), "info_image", "--image", str(image_path)],
        capture=True
    )
    
    output = info_proc.stdout.strip()
    info: Dict[str, Any] = {}
    props_args: List[str] = []

    partition_size_match = re.search(r"^Image size:\s*(\d+)\s*bytes", output, re.MULTILINE)
    if partition_size_match:
        info['partition_size'] = partition_size_match.group(1)
    
    data_size_match = re.search(r"Original image size:\s*(\d+)\s*bytes", output)
    if data_size_match:
        info['data_size'] = data_size_match.group(1)
    else:
        desc_size_match = re.search(r"^\s*Image Size:\s*(\d+)\s*bytes", output, re.MULTILINE)
        if desc_size_match:
            info['data_size'] = desc_size_match.group(1)

    patterns = {
        'name': r"Partition Name:\s*(\S+)",
        'salt': r"Salt:\s*([0-9a-fA-F]+)",
        'algorithm': r"Algorithm:\s*(\S+)",
        'pubkey_sha1': r"Public key \(sha1\):\s*([0-9a-fA-F]+)",
    }
    
    header_section = output.split('Descriptors:')[0]
    rollback_match = re.search(r"Rollback Index:\s*(\d+)", header_section)
    if rollback_match:
        info['rollback'] = rollback_match.group(1)
        
    flags_match = re.search(r"Flags:\s*(\d+)", header_section)
    if flags_match:
        info['flags'] = flags_match.group(1)
        if output: 
            print(get_string("img_info_flags").format(flags=info['flags']))
        
    for key, pattern in patterns.items():
        if key not in info:
            match = re.search(pattern, output)
            if match:
                info[key] = match.group(1)

    for line in output.split('\n'):
        if line.strip().startswith("Prop:"):
            parts = line.split('->')
            key = parts[0].split(':')[-1].strip()
            val = parts[1].strip()[1:-1]
            info[key] = val
            props_args.extend(["--prop", f"{key}:{val}"])
            
    info['props_args'] = props_args
    if props_args and output: 
        print(get_string("img_info_props").format(count=len(props_args) // 2))

    return info

def _apply_hash_footer(
    image_path: Path, 
    image_info: Dict[str, Any], 
    key_file: Path, 
    new_rollback_index: Optional[str] = None
) -> None:
    rollback_index = new_rollback_index if new_rollback_index is not None else image_info['rollback']
    
    print(get_string("img_footer_adding").format(name=image_path.name))
    print(get_string("img_footer_details").format(part=image_info['name'], rb=rollback_index))

    add_footer_cmd = [
        str(const.PYTHON_EXE), str(const.AVBTOOL_PY), "add_hash_footer",
        "--image", str(image_path), 
        "--key", str(key_file),
        "--algorithm", image_info['algorithm'], 
        "--partition_size", image_info['partition_size'],
        "--partition_name", image_info['name'], 
        "--rollback_index", str(rollback_index),
        "--salt", image_info['salt'], 
        *image_info.get('props_args', [])
    ]
    
    if 'flags' in image_info:
        add_footer_cmd.extend(["--flags", image_info.get('flags', '0')])
        print(get_string("img_footer_restore_flags").format(flags=image_info.get('flags', '0')))

    utils.run_command(add_footer_cmd)
    print(get_string("img_footer_success").format(name=image_path.name))

def patch_chained_image_rollback(
    image_name: str, 
    current_rb_index: int, 
    new_image_path: Path, 
    patched_image_path: Path
) -> None:
    try:
        print(get_string("img_analyze_new").format(name=image_name))
        info = extract_image_avb_info(new_image_path)
        new_rb_index = int(info.get('rollback', '0'))
        print(get_string("img_new_index").format(index=new_rb_index))

        if new_rb_index >= current_rb_index:
            print(get_string("img_index_ok").format(name=image_name))
            shutil.copy(new_image_path, patched_image_path)
            return

        print(get_string("img_patch_bypass").format(name=image_name, old=new_rb_index, new=current_rb_index))
        
        for key in ['partition_size', 'name', 'salt', 'algorithm', 'pubkey_sha1']:
            if key not in info:
                raise KeyError(get_string("img_err_missing_key").format(key=key, name=new_image_path.name))
        
        key_file = const.KEY_MAP.get(info['pubkey_sha1']) 
        if not key_file:
            raise KeyError(get_string("img_err_unknown_key").format(key=info['pubkey_sha1'], name=new_image_path.name))
        
        shutil.copy(new_image_path, patched_image_path)
        
        _apply_hash_footer(
            image_path=patched_image_path,
            image_info=info,
            key_file=key_file,
            new_rollback_index=str(current_rb_index)
        )

    except (KeyError, subprocess.CalledProcessError, FileNotFoundError) as e:
        print(get_string("img_err_processing").format(name=image_name, e=e), file=sys.stderr)
        raise

def patch_vbmeta_image_rollback(
    image_name: str, 
    current_rb_index: int, 
    new_image_path: Path, 
    patched_image_path: Path
) -> None:
    try:
        print(get_string("img_analyze_new").format(name=image_name))
        info = extract_image_avb_info(new_image_path)
        new_rb_index = int(info.get('rollback', '0'))
        print(get_string("img_new_index").format(index=new_rb_index))

        if new_rb_index >= current_rb_index:
            print(get_string("img_index_ok").format(name=image_name))
            shutil.copy(new_image_path, patched_image_path)
            return

        print(get_string("img_patch_bypass").format(name=image_name, old=new_rb_index, new=current_rb_index))

        for key in ['algorithm', 'pubkey_sha1']:
            if key not in info:
                raise KeyError(get_string("img_err_missing_key").format(key=key, name=new_image_path.name))
        
        key_file = const.KEY_MAP.get(info['pubkey_sha1']) 
        if not key_file:
            raise KeyError(get_string("img_err_unknown_key").format(key=info['pubkey_sha1'], name=new_image_path.name))

        remake_cmd = [
            str(const.PYTHON_EXE), str(const.AVBTOOL_PY), "make_vbmeta_image",
            "--output", str(patched_image_path),
            "--key", str(key_file),
            "--algorithm", info['algorithm'],
            "--rollback_index", str(current_rb_index),
            "--flags", info.get('flags', '0'),
            "--include_descriptors_from_image", str(new_image_path)
        ]
        
        utils.run_command(remake_cmd)
        print(get_string("img_patch_success").format(name=image_name))

    except (KeyError, subprocess.CalledProcessError, FileNotFoundError) as e:
        print(get_string("img_err_processing").format(name=image_name, e=e), file=sys.stderr)
        raise

def process_boot_image_avb(image_to_process: Path) -> None:
    print(get_string("img_verify_boot")) 
    boot_bak_img = const.BASE_DIR / "boot.bak.img"
    if not boot_bak_img.exists():
        print(get_string("img_err_boot_bak_missing").format(name=boot_bak_img.name), file=sys.stderr)
        raise FileNotFoundError(get_string("img_err_boot_bak_missing").format(name=boot_bak_img.name))
        
    boot_info = extract_image_avb_info(boot_bak_img)
    
    for key in ['partition_size', 'name', 'rollback', 'salt', 'algorithm', 'pubkey_sha1']:
        if key not in boot_info:
            raise KeyError(get_string("img_err_missing_key").format(key=key, name=boot_bak_img.name))
            
    boot_pubkey = boot_info.get('pubkey_sha1')
    key_file = const.KEY_MAP.get(boot_pubkey) 
    
    if not key_file:
        print(get_string("img_err_boot_key_mismatch").format(key=boot_pubkey))
        raise KeyError(get_string("img_err_boot_key_mismatch").format(key=boot_pubkey))

    print(get_string("img_key_matched").format(name=key_file.name))
    
    _apply_hash_footer(
        image_path=image_to_process,
        image_info=boot_info,
        key_file=key_file
    )