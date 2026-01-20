import sys
import os
import pytest

sys.path.append(os.path.abspath(os.path.join(os.path.dirname(__file__), '../bin')))

def test_import_main_module():
    try:
        from ltbox import main
    except ImportError as e:
        pytest.fail(f"Failed to import ltbox.main: {e}")

    assert hasattr(main, "CommandRegistry"), "CommandRegistry class is missing in ltbox.main"
    assert hasattr(main, "setup_console"), "setup_console function is missing in ltbox.main"

def test_import_utils_module():
    try:
        from ltbox import utils
    except ImportError as e:
        pytest.fail(f"Failed to import ltbox.utils: {e}")

    assert hasattr(utils, "run_command"), "run_command function is missing in ltbox.utils"
    assert hasattr(utils, "temporary_workspace"), "temporary_workspace is missing in ltbox.utils"
