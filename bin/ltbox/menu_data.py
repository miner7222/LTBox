from dataclasses import dataclass
from typing import List, Optional

from .i18n import get_string


@dataclass(frozen=True)
class MenuItem:
    item_type: str
    key: Optional[str] = None
    text: str = ""
    action: Optional[str] = None

    @classmethod
    def option(cls, key: str, text: str, action: Optional[str] = None) -> "MenuItem":
        return cls(item_type="option", key=key, text=text, action=action)

    @classmethod
    def label(cls, text: str) -> "MenuItem":
        return cls(item_type="label", text=text)

    @classmethod
    def separator(cls) -> "MenuItem":
        return cls(item_type="separator")


def get_advanced_menu_data(target_region: str) -> List[MenuItem]:
    region_text = (
        get_string("menu_adv_1_row")
        if target_region == "ROW"
        else get_string("menu_adv_1_prc")
    )

    return [
        MenuItem.label(get_string("menu_adv_sub_region_dump")),
        MenuItem.option("1", region_text, action="convert"),
        MenuItem.separator(),
        MenuItem.label(get_string("menu_adv_sub_patch_region")),
        MenuItem.option("2", get_string("menu_adv_2"), action="dump_partitions"),
        MenuItem.option("3", get_string("menu_adv_3"), action="edit_dp"),
        MenuItem.option("4", get_string("menu_adv_4"), action="flash_partitions"),
        MenuItem.separator(),
        MenuItem.label(get_string("menu_adv_sub_arb")),
        MenuItem.option("5", get_string("menu_adv_5"), action="read_anti_rollback"),
        MenuItem.option("6", get_string("menu_adv_6"), action="patch_anti_rollback"),
        MenuItem.option("7", get_string("menu_adv_7"), action="write_anti_rollback"),
        MenuItem.separator(),
        MenuItem.label(get_string("menu_adv_sub_xml_flash")),
        MenuItem.option("8", get_string("menu_adv_8"), action="decrypt_xml"),
        MenuItem.option(
            "9", get_string("task_title_modify_xml_wipe"), action="modify_xml_wipe"
        ),
        MenuItem.option(
            "10", get_string("task_title_modify_xml_nowipe"), action="modify_xml"
        ),
        MenuItem.option("11", get_string("menu_adv_11"), action="flash_full_firmware"),
        MenuItem.separator(),
        MenuItem.label(get_string("menu_adv_sub_nav")),
        MenuItem.option("b", get_string("menu_back"), action="back"),
        MenuItem.option("x", get_string("menu_main_exit"), action="exit"),
    ]


def get_root_mode_menu_data() -> List[MenuItem]:
    return [
        MenuItem.option("1", get_string("menu_root_mode_1")),
        MenuItem.option("2", get_string("menu_root_mode_2")),
        MenuItem.separator(),
        MenuItem.option("b", get_string("menu_back")),
        MenuItem.option("x", get_string("menu_main_exit")),
    ]


def get_root_menu_data(gki: bool, root_type: str) -> List[MenuItem]:
    items: List[MenuItem] = []
    if gki:
        items.append(
            MenuItem.option(
                "1", get_string("menu_root_1_gki"), action="root_device_gki"
            )
        )
        items.append(
            MenuItem.option(
                "2", get_string("menu_root_2_gki"), action="patch_root_image_file_gki"
            )
        )
    else:
        label_2 = get_string("menu_root_2_lkm")
        if root_type == "sukisu":
            label_2 = label_2.replace("KernelSU Next", "SukiSU Ultra")
        elif root_type == "magisk":
            label_2 = label_2.replace("KernelSU Next", "Magisk")
        items.append(
            MenuItem.option(
                "1", get_string("menu_root_1_lkm"), action="root_device_lkm"
            )
        )
        items.append(MenuItem.option("2", label_2, action="patch_root_image_file_lkm"))

    items.append(MenuItem.separator())
    items.append(MenuItem.option("b", get_string("menu_back"), action="back"))
    items.append(MenuItem.option("m", get_string("menu_root_m"), action="return"))
    items.append(MenuItem.option("x", get_string("menu_main_exit"), action="exit"))
    return items


def get_settings_menu_data(
    skip_adb_state: str, skip_rb_state: str, target_region: str
) -> List[MenuItem]:
    region_label = (
        get_string("menu_settings_device_row")
        if target_region == "ROW"
        else get_string("menu_settings_device_prc")
    )

    return [
        MenuItem.option("1", region_label, action="toggle_region"),
        MenuItem.option(
            "2",
            get_string("menu_settings_skip_adb").format(state=skip_adb_state),
            action="toggle_adb",
        ),
        MenuItem.option(
            "3",
            get_string("menu_settings_skip_rb").format(state=skip_rb_state),
            action="toggle_rollback",
        ),
        MenuItem.option("4", get_string("menu_settings_lang"), action="change_lang"),
        MenuItem.option(
            "5", get_string("menu_settings_check_update"), action="check_update"
        ),
        MenuItem.separator(),
        MenuItem.option("b", get_string("menu_back"), action="back"),
    ]


def get_main_menu_data(target_region: str) -> List[MenuItem]:
    if target_region == "ROW":
        install_wipe_text = get_string("menu_main_install_wipe_row")
        install_keep_text = get_string("menu_main_install_keep_row")
    else:
        install_wipe_text = get_string("menu_main_install_wipe_prc")
        install_keep_text = get_string("menu_main_install_keep_prc")

    return [
        MenuItem.option("1", install_wipe_text, action="patch_all_wipe"),
        MenuItem.option("2", install_keep_text, action="patch_all"),
        MenuItem.separator(),
        MenuItem.option("3", get_string("menu_main_rescue"), action="rescue_ota"),
        MenuItem.option("4", get_string("menu_main_disable_ota"), action="disable_ota"),
        MenuItem.separator(),
        MenuItem.option("5", get_string("menu_main_root"), action="menu_root"),
        MenuItem.option("6", get_string("menu_main_unroot"), action="unroot_device"),
        MenuItem.option(
            "7", get_string("menu_main_rec_flash"), action="sign_and_flash_twrp"
        ),
        MenuItem.separator(),
        MenuItem.option("0", get_string("menu_settings_title"), action="menu_settings"),
        MenuItem.option("a", get_string("menu_main_adv"), action="menu_advanced"),
        MenuItem.option("x", get_string("menu_main_exit"), action="exit"),
    ]
