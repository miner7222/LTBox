import shutil
import sys
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Optional, List, Dict, Any

from .. import constants as const
from .. import utils
from ..crypto import decrypt_file
from ..i18n import get_string

def decrypt_x_files() -> None:
    print(get_string("act_start_decrypt_xml"))
    
    print(get_string("act_wait_image"))
    prompt = get_string("act_prompt_image")
    utils.wait_for_directory(const.IMAGE_DIR, prompt)

    if const.OUTPUT_XML_DIR.exists():
        shutil.rmtree(const.OUTPUT_XML_DIR)
    const.OUTPUT_XML_DIR.mkdir(parents=True, exist_ok=True)

    print(get_string("img_xml_scan"))
    
    x_files = list(const.IMAGE_DIR.glob("*.x"))
    xml_files = list(const.IMAGE_DIR.glob("*.xml"))
    
    processed_files = False

    if x_files:
        print(get_string("img_xml_found_x").format(count=len(x_files), dir=const.OUTPUT_XML_DIR.name))
        for file in x_files:
            out_file = const.OUTPUT_XML_DIR / file.with_suffix('.xml').name
            try:
                if decrypt_file(str(file), str(out_file)):
                    print(get_string("img_xml_decrypt_ok").format(src=file.name, dst=out_file.name))
                    processed_files = True
                else:
                    print(get_string("img_xml_decrypt_fail").format(name=file.name))
            except Exception as e:
                print(get_string("img_xml_decrypt_err").format(name=file.name, e=e), file=sys.stderr)

    if xml_files:
        print(get_string("img_xml_found_xml").format(count=len(xml_files), dir=const.OUTPUT_XML_DIR.name))
        for file in xml_files:
            out_file = const.OUTPUT_XML_DIR / file.name
            try:
                if out_file.exists():
                    out_file.unlink()
                shutil.move(str(file), str(out_file))
                print(get_string("img_xml_moved").format(name=file.name))
                processed_files = True
            except Exception as e:
                print(get_string("img_xml_move_err").format(name=file.name, e=e), file=sys.stderr)

    if not processed_files:
        print(get_string("img_xml_no_files").format(dir=const.IMAGE_DIR.name))
        shutil.rmtree(const.OUTPUT_XML_DIR)
        raise FileNotFoundError(get_string("img_xml_no_files").format(dir=const.IMAGE_DIR.name))
    
    print("\n  " + "=" * 58)
    print(get_string("act_success"))
    print(get_string("act_xml_ready").format(dir=const.OUTPUT_XML_DIR.name))
    print("  " + "=" * 58)

def _is_garbage_file(path: Path) -> bool:
    name = path.name.lower()
    stem = path.stem.lower()
    if stem == "rawprogram_unsparse0": return True
    if "wipe_partitions" in name or "blank_gpt" in name: return True
    return False

def _ensure_rawprogram4(output_dir: Path) -> None:
    rawprogram4 = output_dir / "rawprogram4.xml"
    rawprogram_unsparse4 = output_dir / "rawprogram_unsparse4.xml"
    
    if not rawprogram4.exists() and rawprogram_unsparse4.exists():
        print(get_string("img_xml_copy_raw4"))
        try:
            tree = ET.parse(rawprogram_unsparse4)
            root = tree.getroot()
            
            devinfo_modified = False
            for prog in root.findall('program'):
                if prog.get('label', '').lower() == 'devinfo':
                    if 'devinfo.img' in prog.get('filename', '').lower():
                        prog.set('filename', '')
                        devinfo_modified = True
            
            tree.write(rawprogram4, encoding='utf-8', xml_declaration=True)
            
            if devinfo_modified:
                print(get_string("img_xml_created_raw4_devinfo").format(name=rawprogram4.name))
            else:
                print(get_string("img_xml_created_raw4_no_devinfo").format(name=rawprogram4.name))
                
        except Exception as e:
            print(get_string("img_xml_err_process_raw4").format(name=rawprogram_unsparse4.name, e=e), file=sys.stderr)
            print(get_string("img_xml_fallback_copy"))
            shutil.copy(rawprogram_unsparse4, rawprogram4)

def _ensure_rawprogram_save_persist(output_dir: Path) -> Path:
    print(get_string("img_xml_mod_raw"))
    rawprogram_save = output_dir / "rawprogram_save_persist_unsparse0.xml"

    if rawprogram_save.exists():
        return rawprogram_save

    rawprogram_fallback = output_dir / "rawprogram_unsparse0-half.xml"
    
    if rawprogram_fallback.exists():
        print(get_string("img_xml_rename_fallback").format(target=rawprogram_save.name, src=rawprogram_fallback.name))
        try:
            rawprogram_fallback.rename(rawprogram_save)
            return rawprogram_save
        except OSError as e:
            print(get_string("img_xml_rename_err").format(e=e), file=sys.stderr)
            raise
    else:
        fallback_candidates = ["rawprogram_unsparse0.xml", "rawprogram0.xml"]
        
        for cand_name in fallback_candidates:
            cand_path = output_dir / cand_name
            if cand_path.exists():
                print(get_string("img_xml_fallback_found").format(src=cand_path.name, dst=rawprogram_save.name))
                try:
                    tree = ET.parse(cand_path)
                    root = tree.getroot()
                    
                    persist_found = False
                    for prog in root.findall('program'):
                        if prog.get('label', '').lower() == 'persist':
                            prog.set('filename', '')
                            persist_found = True
                    
                    tree.write(rawprogram_save, encoding='utf-8', xml_declaration=True)
                    
                    if persist_found:
                        print(get_string("img_xml_created_save_persist").format(name=rawprogram_save.name))
                    else:
                        print(get_string("img_xml_warn_persist_missing").format(name=cand_path.name))
                    
                    return rawprogram_save

                except Exception as e:
                    print(get_string("img_xml_err_process_fallback").format(name=cand_path.name, e=e), file=sys.stderr)
                    raise

        msg = get_string("img_xml_critical_missing").format(f1=rawprogram_save.name, f2=rawprogram_fallback.name)
        print(msg)
        print(get_string("img_xml_abort_mod"))
        raise FileNotFoundError(msg)

def _patch_xml_for_wipe(xml_path: Path, wipe: int) -> None:
    try:
        tree = ET.parse(xml_path)
        root = tree.getroot()
        
        if wipe == 0:
            print(get_string("img_xml_nowipe"))
            for prog in root.findall('program'):
                label = prog.get('label', '').lower()
                if label.startswith('metadata') or label.startswith('userdata'):
                    prog.set('filename', '')
        else:
            print(get_string("img_xml_wipe"))
            
        tree.write(xml_path, encoding='utf-8', xml_declaration=True)
        print(get_string("img_xml_patch_ok"))
    except Exception as e:
        print(get_string("img_xml_patch_err").format(e=e), file=sys.stderr)
        raise

def _cleanup_garbage_xmls(output_dir: Path) -> None:
    print(get_string("img_xml_cleanup"))
    
    files_to_delete = []
    for f in output_dir.glob("*.xml"):
        if _is_garbage_file(f):
            files_to_delete.append(f)

    if files_to_delete:
        for f in files_to_delete:
            try:
                f.unlink()
                print(get_string("img_xml_deleted").format(name=f.name))
            except OSError as e:
                print(get_string("img_xml_del_err").format(name=f.name, e=e))
    else:
        print(get_string("img_xml_no_del"))

def _modify_xml_algo(output_dir: Path, wipe: int = 0) -> None:
    _ensure_rawprogram4(output_dir)
    
    rawprogram_save = _ensure_rawprogram_save_persist(output_dir)
    
    _patch_xml_for_wipe(rawprogram_save, wipe)
    
    _cleanup_garbage_xmls(output_dir)

    print(get_string("img_xml_complete").format(dir=output_dir.name))

def _create_write_xml(
    src_xml_path: Path, 
    dest_xml_path: Path, 
    target_label: str, 
    new_filename: str, 
    success_key: str, 
    error_key: str, 
    warn_file_missing_key: str,
    warn_label_missing_key: str
) -> None:
    if not src_xml_path.exists():
        print(get_string(warn_file_missing_key).format(name=src_xml_path.name))
        return

    try:
        tree = ET.parse(src_xml_path)
        root = tree.getroot()
        modified = False
        for prog in root.findall('program'):
            if prog.get('label', '').lower() == target_label:
                prog.set('filename', new_filename)
                modified = True
        
        tree.write(dest_xml_path, encoding='utf-8', xml_declaration=True)
        
        if modified:
            print(get_string(success_key).format(name=dest_xml_path.name, parent=dest_xml_path.parent.name))
        else:
            print(get_string(warn_label_missing_key).format(name=src_xml_path.name))
    except Exception as e:
        print(get_string(error_key).format(name=dest_xml_path.name, e=e), file=sys.stderr)

def modify_xml(wipe: int = 0, skip_dp: bool = False) -> None:
    print(get_string("act_start_xml_mod"))
    
    if not const.OUTPUT_XML_DIR.exists() or not any(const.OUTPUT_XML_DIR.iterdir()):
        print(get_string("act_err_no_xml_output_folder").format(dir=const.OUTPUT_XML_DIR.name), file=sys.stderr)
        print(get_string("act_err_run_decrypt_first"), file=sys.stderr)
        raise FileNotFoundError(get_string("act_err_run_decrypt_first"))

    with utils.temporary_workspace(const.WORKING_DIR):
        print(get_string("act_create_temp").format(dir=const.WORKING_DIR.name))
        try:
            _modify_xml_algo(const.OUTPUT_XML_DIR, wipe=wipe)

            if not skip_dp:
                print(get_string("act_create_write_xml"))

                _create_write_xml(
                    src_xml_path=(const.OUTPUT_XML_DIR / "rawprogram_save_persist_unsparse0.xml"),
                    dest_xml_path=(const.OUTPUT_XML_DIR / "rawprogram_write_persist_unsparse0.xml"),
                    target_label='persist',
                    new_filename='persist.img',
                    success_key="act_created_persist_xml",
                    error_key="act_err_create_persist_xml",
                    warn_file_missing_key="act_warn_persist_xml_missing",
                    warn_label_missing_key="act_warn_persist_label_missing"
                )

                _create_write_xml(
                    src_xml_path=(const.OUTPUT_XML_DIR / "rawprogram4.xml"),
                    dest_xml_path=(const.OUTPUT_XML_DIR / "rawprogram4_write_devinfo.xml"),
                    target_label='devinfo',
                    new_filename='devinfo.img',
                    success_key="act_created_devinfo_xml",
                    error_key="act_err_create_devinfo_xml",
                    warn_file_missing_key="act_warn_devinfo_xml_missing",
                    warn_label_missing_key="act_warn_devinfo_label_missing"
                )

        except Exception as e:
            print(get_string("act_err_xml_mod").format(e=e), file=sys.stderr)
            raise
        
        print(get_string("act_clean_temp").format(dir=const.WORKING_DIR.name))
    
    print("\n  " + "=" * 58)
    print(get_string("act_success"))
    print(get_string("act_xml_ready").format(dir=const.OUTPUT_XML_DIR.name))
    print("  " + "=" * 58)