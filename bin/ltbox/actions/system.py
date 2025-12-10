from typing import Optional
from .. import device
from ..i18n import get_string
from ..errors import ToolError

def detect_active_slot_robust(dev: device.DeviceController) -> Optional[str]:
    try:
        return dev.detect_active_slot()
    except Exception as e:
        raise ToolError(get_string("act_warn_slot_fail")) from e

def disable_ota(dev: device.DeviceController) -> None:
    from .. import utils
    
    utils.ui.echo(get_string("act_start_ota"))
    
    dev.wait_for_adb()

    utils.ui.echo(get_string("act_ota_settings_put"))
    try:
        dev.adb_shell("settings put global ota_disable_automatic_update 1")
        dev.adb_shell("settings put secure lenovo_ota_new_version_found 0")
    except Exception as e:
        utils.ui.echo(f"Warning: Failed to update settings: {e}", err=True)

    packages = [
        "com.lenovo.ota",
        "com.tblenovo.lenovowhatsnew",
        "com.lenovo.tbengine"
    ]

    for pkg in packages:
        try:
            dev.adb_shell(f"pm clear {pkg}")
        except Exception:
            pass

        utils.ui.echo(get_string("act_ota_uninstalling").format(pkg=pkg))
        try:
            output = dev.adb_shell(f"pm uninstall -k --user 0 {pkg}")
            if "Success" in output:
                utils.ui.echo(get_string("act_ota_uninstall_success").format(pkg=pkg))
            else:
                utils.ui.echo(get_string("act_ota_uninstall_fail").format(pkg=pkg))
        except Exception:
            utils.ui.echo(get_string("act_ota_uninstall_fail").format(pkg=pkg))

    utils.ui.echo(get_string("act_ota_finished"))