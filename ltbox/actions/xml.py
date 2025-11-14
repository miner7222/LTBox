import re
import shutil
import sys
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Optional, List, Dict, Any

from .. import constants as const
from .. import utils
from ..crypto import decrypt_file
from ..i18n import get_string

def _modify_xml_algo(wipe: int = 0) -> None:
    def is_garbage_file(path: Path) -> bool:
        name = path.name.lower()
        stem = path.stem.lower()
        if stem == "rawprogram_unsparse0": return True
        if "wipe_partitions" in name or "blank_gpt" in name: return True
        return False

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
        raise FileNotFoundError(f"No .x or .xml files in {const.IMAGE_DIR.name}")

    rawprogram4 = const.OUTPUT_XML_DIR / "rawprogram4.xml"
    rawprogram_unsparse4 = const.OUTPUT_XML_DIR / "rawprogram_unsparse4.xml"
    
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
                print(f"  > Created {rawprogram4.name} with 'devinfo' filename cleared.")
            else:
                print(f"  > Created {rawprogram4.name} (no 'devinfo.img' found/modified).")
                
        except Exception as e:
            print(f"[!] Error processing {rawprogram_unsparse4.name}: {e}", file=sys.stderr)
            print("  > Fallback: Simply copying file.")
            shutil.copy(rawprogram_unsparse4, rawprogram4)

    print(get_string("img_xml_mod_raw"))

    rawprogram_save = const.OUTPUT_XML_DIR / "rawprogram_save_persist_unsparse0.xml"

    if not rawprogram_save.exists():
        rawprogram_fallback = const.OUTPUT_XML_DIR / "rawprogram_unsparse0-half.xml"
        
        if rawprogram_fallback.exists():
            print(get_string("img_xml_rename_fallback").format(target=rawprogram_save.name, src=rawprogram_fallback.name))
            try:
                rawprogram_fallback.rename(rawprogram_save)
            except OSError as e:
                print(get_string("img_xml_rename_err").format(e=e), file=sys.stderr)
                raise
        else:
            fallback_candidates = ["rawprogram_unsparse0.xml", "rawprogram0.xml"]
            found_candidate = None
            
            for cand_name in fallback_candidates:
                cand_path = const.OUTPUT_XML_DIR / cand_name
                if cand_path.exists():
                    found_candidate = cand_path
                    break
            
            if found_candidate:
                print(f"[*] Fallback found: {found_candidate.name}. Creating {rawprogram_save.name}...")
                try:
                    tree = ET.parse(found_candidate)
                    root = tree.getroot()
                    
                    persist_found = False
                    for prog in root.findall('program'):
                        if prog.get('label', '').lower() == 'persist':
                            prog.set('filename', '')
                            persist_found = True
                    
                    tree.write(rawprogram_save, encoding='utf-8', xml_declaration=True)
                    
                    if persist_found:
                        print(f"  > Created {rawprogram_save.name} (persist filename cleared).")
                    else:
                        print(f"  [!] Warning: 'persist' label not found in {found_candidate.name}. Created anyway.")

                except Exception as e:
                    print(f"[!] Error processing fallback {found_candidate.name}: {e}", file=sys.stderr)
                    raise
            else:
                print(get_string("img_xml_critical_missing").format(f1=rawprogram_save.name, f2=rawprogram_fallback.name))
                print(get_string("img_xml_abort_mod"))
                raise FileNotFoundError(f"Critical XML file missing: {rawprogram_save.name} or fallbacks")

    try:
        with open(rawprogram_save, 'r', encoding='utf-8') as f:
            content = f.read()
        
        if wipe == 0:
            print(get_string("img_xml_nowipe"))
            for i in range(1, 11):
                content = content.replace(f'filename="metadata_{i}.img"', '')
            for i in range(1, 21):
                content = content.replace(f'filename="userdata_{i}.img"', '')
        else:
            print(get_string("img_xml_wipe"))
            
        with open(rawprogram_save, 'w', encoding='utf-8') as f:
            f.write(content)
        print(get_string("img_xml_patch_ok"))
    except Exception as e:
        print(get_string("img_xml_patch_err").format(e=e), file=sys.stderr)
        raise

    print(get_string("img_xml_cleanup"))
    
    files_to_delete = []
    for f in const.OUTPUT_XML_DIR.glob("*.xml"):
        if is_garbage_file(f):
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

    print(get_string("img_xml_complete").format(dir=const.OUTPUT_XML_DIR.name))


def modify_xml(wipe: int = 0, skip_dp: bool = False) -> None:
    print(get_string("act_start_xml_mod"))
    
    print(get_string("act_wait_image"))
    prompt = get_string("act_prompt_image")
    utils.wait_for_directory(const.IMAGE_DIR, prompt)

    if const.OUTPUT_XML_DIR.exists():
        shutil.rmtree(const.OUTPUT_XML_DIR)
    const.OUTPUT_XML_DIR.mkdir(exist_ok=True)

    with utils.temporary_workspace(const.WORKING_DIR):
        print(get_string("act_create_temp").format(dir=const.WORKING_DIR.name))
        try:
            _modify_xml_algo(wipe=wipe)

            if not skip_dp:
                print(get_string("act_create_write_xml"))

                src_persist_xml = const.OUTPUT_XML_DIR / "rawprogram_save_persist_unsparse0.xml"
                dest_persist_xml = const.OUTPUT_XML_DIR / "rawprogram_write_persist_unsparse0.xml"
                
                if src_persist_xml.exists():
                    try:
                        tree = ET.parse(src_persist_xml)
                        root = tree.getroot()
                        modified = False
                        for prog in root.findall('program'):
                            if prog.get('label', '').lower() == 'persist':
                                prog.set('filename', 'persist.img')
                                modified = True
                        
                        tree.write(dest_persist_xml, encoding='utf-8', xml_declaration=True)
                        if modified:
                            print(get_string("act_created_persist_xml").format(name=dest_persist_xml.name, parent=dest_persist_xml.parent.name))
                        else:
                            print(f"[!] Warning: 'persist' label not found in {src_persist_xml.name}")
                    except Exception as e:
                        print(get_string("act_err_create_persist_xml").format(name=dest_persist_xml.name, e=e), file=sys.stderr)
                else:
                    print(get_string("act_warn_persist_xml_missing").format(name=src_persist_xml.name))

                src_devinfo_xml = const.OUTPUT_XML_DIR / "rawprogram4.xml"
                dest_devinfo_xml = const.OUTPUT_XML_DIR / "rawprogram4_write_devinfo.xml"
                
                if src_devinfo_xml.exists():
                    try:
                        tree = ET.parse(src_devinfo_xml)
                        root = tree.getroot()
                        modified = False
                        for prog in root.findall('program'):
                            if prog.get('label', '').lower() == 'devinfo':
                                prog.set('filename', 'devinfo.img')
                                modified = True

                        tree.write(dest_devinfo_xml, encoding='utf-8', xml_declaration=True)
                        if modified:
                            print(get_string("act_created_devinfo_xml").format(name=dest_devinfo_xml.name, parent=dest_devinfo_xml.parent.name))
                        else:
                            print(f"[!] Warning: 'devinfo' label not found in {src_devinfo_xml.name}")
                    except Exception as e:
                        print(get_string("act_err_create_devinfo_xml").format(name=dest_devinfo_xml.name, e=e), file=sys.stderr)
                else:
                    print(get_string("act_warn_devinfo_xml_missing").format(name=src_devinfo_xml.name))

        except Exception as e:
            print(get_string("act_err_xml_mod").format(e=e), file=sys.stderr)
            raise
        
        print(get_string("act_clean_temp").format(dir=const.WORKING_DIR.name))
    
    print("\n" + "=" * 61)
    print(get_string("act_success"))
    print(get_string("act_xml_ready").format(dir=const.OUTPUT_XML_DIR.name))
    print(get_string("act_xml_next_step"))
    print("=" * 61)