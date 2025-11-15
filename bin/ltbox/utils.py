import os
import platform
import subprocess
import sys
import shutil
from contextlib import contextmanager
from datetime import datetime
from pathlib import Path
from typing import List, Optional, Callable, Generator, Any, Union, Dict, Tuple

from . import constants as const
from .i18n import get_string

class ConsoleUI:
    def echo(self, message: str = "", err: bool = False) -> None:
        dest = sys.stderr if err else sys.stdout
        print(message, file=dest)

    def prompt(self, message: str = "") -> str:
        return input(message)

    def clear(self) -> None:
        if platform.system() == "Windows":
            os.system('cls')
        else:
            os.system('clear')

ui = ConsoleUI()

_CACHED_ENV = None

def _get_tool_env() -> dict:
    global _CACHED_ENV
    if _CACHED_ENV is None:
        _CACHED_ENV = os.environ.copy()
        paths = [str(const.TOOLS_DIR), str(const.DOWNLOAD_DIR)]
        _CACHED_ENV['PATH'] = os.pathsep.join(paths) + os.pathsep + _CACHED_ENV['PATH']
    return _CACHED_ENV

def run_command(
    command: Union[List[str], str], 
    shell: bool = False, 
    check: bool = True, 
    env: Optional[dict] = None, 
    capture: bool = False,
    cwd: Optional[Union[str, Path]] = None
) -> subprocess.CompletedProcess:
    run_env = env if env is not None else _get_tool_env()

    return subprocess.run(
        command, shell=shell, check=check, capture_output=capture,
        text=True, encoding='utf-8', errors='ignore', env=run_env, cwd=cwd
    )

def get_platform_executable(name: str) -> Path:
    system = platform.system()
    executables = {
        "Windows": f"{name}.exe",
        "Linux": f"{name}-linux",
        "Darwin": f"{name}-macos"
    }
    exe_name = executables.get(system)
    if not exe_name:
        raise RuntimeError(get_string("err_unsupported_os").format(system=system))
    return const.DOWNLOAD_DIR / exe_name

def _wait_for_resource(
    target_path: Path, 
    check_func: Callable[[Path, Optional[List[str]]], bool], 
    prompt_msg: str, 
    item_list: Optional[List[str]] = None
) -> bool:
    target_path.mkdir(exist_ok=True, parents=True)
    while True:
        if check_func(target_path, item_list):
            return True
        
        ui.clear()
            
        ui.echo(get_string('utils_wait_resource'))
        ui.echo(prompt_msg)
        if item_list:
            ui.echo(get_string('utils_missing_items'))
            for item in item_list:
                if not (target_path / item).exists():
                    ui.echo(f" - {item}")
        
        ui.echo(get_string('utils_press_enter'))
        try:
            ui.prompt()
        except EOFError:
            raise RuntimeError(get_string('process_cancelled'))

def wait_for_files(directory: Path, required_files: List[str], prompt_message: str) -> bool:
    return _wait_for_resource(
        directory, 
        lambda p, f: all((p / i).exists() for i in (f or [])), 
        prompt_message, 
        required_files
    )

def wait_for_directory(directory: Path, prompt_message: str) -> bool:
    return _wait_for_resource(
        directory, 
        lambda p, _: p.is_dir() and any(p.iterdir()), 
        prompt_message,
        None
    )

def check_dependencies() -> None:
    ui.echo(get_string('utils_check_deps'))
    dependencies = {
        "Python Environment": const.PYTHON_EXE,
        "ADB": const.ADB_EXE,
        "Fastboot": const.FASTBOOT_EXE,
        "avbtool": const.AVBTOOL_PY,
        "fetch tool": get_platform_executable("fetch")
    }
    
    for path in const.KEY_MAP.values():
        dependencies[path.name] = path

    missing_deps = [name for name, path in dependencies.items() if not Path(path).exists()]

    if missing_deps:
        for name in missing_deps:
            ui.echo(get_string('utils_missing_dep').format(name=name))
        ui.echo(get_string('utils_run_install'))
        raise RuntimeError(get_string('utils_run_install'))

    ui.echo(get_string('utils_deps_found'))

@contextmanager
def temporary_workspace(path: Path) -> Generator[Path, None, None]:
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True)
    try:
        yield path
    finally:
        if path.exists():
            try:
                shutil.rmtree(path)
            except OSError as e:
                ui.echo(get_string("warn_failed_cleanup_workspace").format(path=path, e=e), err=True)

def clean_workspace() -> None:
    ui.echo(get_string('utils_cleaning_title'))
    ui.echo(get_string('utils_cleaning_warning'))
    ui.echo("-" * 50)

    folders_to_remove = [
        const.INPUT_CURRENT_DIR, const.INPUT_NEW_DIR,
        const.OUTPUT_DIR, const.OUTPUT_ROOT_DIR, const.OUTPUT_DP_DIR, const.OUTPUT_ANTI_ROLLBACK_DIR,
        const.WORK_DIR,
        const.IMAGE_DIR,
        const.WORKING_DIR,
        const.OUTPUT_XML_DIR,
    ]
    
    ui.echo(get_string('utils_removing_dirs'))
    for folder in folders_to_remove:
        if folder.exists():
            try:
                shutil.rmtree(folder)
                ui.echo(get_string('utils_removed').format(name=f"{folder.name}{os.sep}"))
            except OSError as e:
                ui.echo(get_string('utils_remove_error').format(name=folder.name, e=e), err=True)
        else:
            ui.echo(get_string('utils_skipping').format(name=f"{folder.name}{os.sep}"))

    ui.echo(get_string('utils_cleaning_dl'))
    dl_files_to_remove = [
        "*.zip",
        "*.tar.gz",
    ]
    
    cleaned_dl_files = 0
    for pattern in dl_files_to_remove:
        for f in const.DOWNLOAD_DIR.glob(pattern):
            try:
                f.unlink()
                ui.echo(get_string('utils_removed_temp').format(name=f.name))
                cleaned_dl_files += 1
            except OSError as e:
                ui.echo(get_string('utils_remove_error').format(name=f.name, e=e), err=True)

    if cleaned_dl_files == 0:
        ui.echo(get_string('utils_no_temp_dl'))


    ui.echo(get_string('utils_cleaning_root'))
    file_patterns_to_remove = [
        "*.bak.img",
        "*.root.img",
        "*prc.img",
        "*modified.img",
        "image_info_*.txt",
        "KernelSU*.apk",
        "devinfo.img", 
        "persist.img", 
        "boot.img", 
        "vbmeta.img",
        "platform-tools.zip"
    ]
    
    cleaned_root_files = 0
    for pattern in file_patterns_to_remove:
        for f in const.BASE_DIR.glob(pattern):
            try:
                f.unlink()
                ui.echo(get_string('utils_removed').format(name=f.name))
                cleaned_root_files += 1
            except OSError as e:
                ui.echo(get_string('utils_remove_error').format(name=f.name, e=e), err=True)
    
    if cleaned_root_files == 0:
        ui.echo(get_string('utils_no_temp_root'))

    ui.echo(get_string('utils_clean_complete'))

def _process_binary_file(
    input_path: Union[str, Path], 
    output_path: Union[str, Path], 
    patch_func: Any, 
    copy_if_unchanged: bool = True,
    **kwargs: Any
) -> bool:
    input_path = Path(input_path)
    output_path = Path(output_path)
    
    if not input_path.exists():
        ui.echo(get_string("img_proc_err_not_found").format(path=input_path), err=True)
        return False

    try:
        content = input_path.read_bytes()
        modified_content, stats = patch_func(content, **kwargs)

        if stats.get('changed', False):
            output_path.write_bytes(modified_content)
            ui.echo(get_string("img_proc_success").format(msg=stats.get('message', 'Modifications applied.')))
            ui.echo(get_string("img_proc_saved").format(name=output_path.name))
            return True
        else:
            ui.echo(get_string("img_proc_no_change").format(name=input_path.name, msg=stats.get('message', 'No patterns found')))
            if copy_if_unchanged:
                ui.echo(get_string("img_proc_copying").format(name=output_path.name))
                if input_path != output_path:
                    shutil.copy(input_path, output_path)
                return True
            return False

    except Exception as e:
        ui.echo(get_string("img_proc_error").format(name=input_path.name, e=e), err=True)
        return False