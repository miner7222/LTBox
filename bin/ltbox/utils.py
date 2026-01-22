import json
import os
import shutil
import subprocess
import urllib.request
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Callable, Generator, List, Optional, Union

from . import constants as const
from .i18n import get_string
from .logger import get_logger
from .ui import ui

logger = get_logger()

_CACHED_ENV = None


def get_latest_release_version(repo_owner: str, repo_name: str) -> Optional[str]:
    url = f"https://api.github.com/repos/{repo_owner}/{repo_name}/releases/latest"
    try:
        with urllib.request.urlopen(url, timeout=5) as response:
            if response.status == 200:
                data = json.loads(response.read().decode())
                tag = data.get("tag_name")
                return tag
    except Exception:
        return None
    return None


def is_update_available(current: str, latest: str) -> bool:
    def version_to_tuple(v_str):
        try:
            return tuple(map(int, v_str.lstrip("v").split(".")))
        except ValueError:
            return (0, 0, 0)

    return version_to_tuple(latest) > version_to_tuple(current)


def _get_tool_env() -> dict:
    global _CACHED_ENV
    if _CACHED_ENV is None:
        _CACHED_ENV = os.environ.copy()
        paths = [str(const.TOOLS_DIR), str(const.DOWNLOAD_DIR)]
        _CACHED_ENV["PATH"] = os.pathsep.join(paths) + os.pathsep + _CACHED_ENV["PATH"]
    return _CACHED_ENV


def _run_command_capture(
    command: Union[List[str], str],
    shell: bool,
    check: bool,
    env: dict,
    cwd: Optional[Union[str, Path]],
) -> subprocess.CompletedProcess:
    return subprocess.run(
        command,
        shell=shell,
        check=check,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="ignore",
        env=env,
        cwd=cwd,
    )


def _run_command_stream(
    command: Union[List[str], str],
    shell: bool,
    check: bool,
    env: dict,
    cwd: Optional[Union[str, Path]],
    on_output: Optional[Callable[[str], None]],
) -> subprocess.CompletedProcess:
    process = subprocess.Popen(
        command,
        shell=shell,
        env=env,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="ignore",
        bufsize=1,
    )

    output_lines = []
    if process.stdout:
        for line in process.stdout:
            if on_output is not None:
                on_output(line)
            else:
                logger.info(line.rstrip())
            output_lines.append(line)

    process.wait()
    returncode = process.returncode

    if check and returncode != 0:
        raise subprocess.CalledProcessError(
            returncode, command, output="".join(output_lines)
        )

    return subprocess.CompletedProcess(
        command, returncode, stdout="".join(output_lines), stderr=None
    )


def run_command(
    command: Union[List[str], str],
    shell: bool = False,
    check: bool = True,
    env: Optional[dict] = None,
    capture: bool = False,
    cwd: Optional[Union[str, Path]] = None,
    on_output: Optional[Callable[[str], None]] = None,
) -> subprocess.CompletedProcess:
    run_env = env if env is not None else _get_tool_env()

    if capture:
        return _run_command_capture(command, shell, check, run_env, cwd)

    return _run_command_stream(command, shell, check, run_env, cwd, on_output)


def get_platform_executable(name: str) -> Path:
    return const.DOWNLOAD_DIR / f"{name}.exe"


def _wait_for_resource(
    target_path: Path,
    check_func: Callable[[Path, Optional[List[str]]], bool],
    prompt_msg: str,
    item_list: Optional[List[str]] = None,
) -> bool:
    target_path.mkdir(exist_ok=True, parents=True)
    while True:
        if check_func(target_path, item_list):
            return True

        ui.clear()

        ui.echo(get_string("utils_wait_resource"))
        ui.echo(prompt_msg)
        if item_list:
            ui.echo(get_string("utils_missing_items"))
            for item in item_list:
                if not (target_path / item).exists():
                    ui.echo(get_string("utils_missing_item_format").format(item=item))

        ui.echo(get_string("press_enter_to_continue"))
        try:
            ui.prompt()
        except EOFError:
            raise RuntimeError(get_string("act_op_cancel"))


def wait_for_files(
    directory: Path, required_files: List[str], prompt_message: str
) -> bool:
    return _wait_for_resource(
        directory,
        lambda p, f: all((p / i).exists() for i in (f or [])),
        prompt_message,
        required_files,
    )


def wait_for_directory(directory: Path, prompt_message: str) -> bool:
    return _wait_for_resource(
        directory, lambda p, _: p.is_dir() and any(p.iterdir()), prompt_message, None
    )


def check_dependencies() -> None:
    dependencies = {
        "Python Environment": const.PYTHON_EXE,
        "ADB": const.ADB_EXE,
        "Fastboot": const.FASTBOOT_EXE,
        "avbtool": const.AVBTOOL_PY,
    }

    for path in const.KEY_MAP.values():
        dependencies[path.name] = path

    missing_deps = [
        name for name, path in dependencies.items() if not Path(path).exists()
    ]

    if missing_deps:
        for name in missing_deps:
            ui.echo(get_string("utils_missing_dep").format(name=name))
        ui.echo(get_string("utils_run_install"))
        raise RuntimeError(get_string("utils_run_install"))

    ui.echo(get_string("utils_deps_found"))


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
                ui.echo(
                    get_string("warn_failed_cleanup_workspace").format(path=path, e=e),
                    err=True,
                )


def _process_binary_file(
    input_path: Union[str, Path],
    output_path: Union[str, Path],
    patch_func: Any,
    copy_if_unchanged: bool = True,
    **kwargs: Any,
) -> bool:
    input_path = Path(input_path)
    output_path = Path(output_path)

    if not input_path.exists():
        ui.echo(get_string("img_proc_err_not_found").format(path=input_path), err=True)
        return False

    try:
        content = input_path.read_bytes()
        modified_content, stats = patch_func(content, **kwargs)

        if stats.get("changed", False):
            output_path.write_bytes(modified_content)
            ui.echo(
                get_string("img_proc_success").format(
                    msg=stats.get("message", get_string("img_proc_msg_modified"))
                )
            )
            ui.echo(get_string("img_proc_saved").format(name=output_path.name))
            return True
        else:
            ui.echo(
                get_string("img_proc_no_change").format(
                    name=input_path.name,
                    msg=stats.get("message", get_string("img_proc_msg_no_patterns")),
                )
            )
            if copy_if_unchanged:
                ui.echo(get_string("img_proc_copying").format(name=output_path.name))
                if input_path != output_path:
                    shutil.copy(input_path, output_path)
                return True
            return False

    except (OSError, IOError) as e:
        ui.echo(
            get_string("img_proc_error").format(name=input_path.name, e=e), err=True
        )
        return False
