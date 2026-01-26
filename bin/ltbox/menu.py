from typing import List, Optional, Tuple

from .i18n import get_string
from .menu_data import MenuItem
from .utils import ui


class TerminalMenu:
    def __init__(self, title: str):
        self.title = title
        self.options: List[Tuple[Optional[str], str, bool]] = []
        self.valid_keys: List[str] = []

    def add_option(self, key: str, text: str) -> None:
        self.options.append((key, text, True))
        self.valid_keys.append(key.lower())

    def add_label(self, text: str) -> None:
        self.options.append((None, text, False))

    def add_separator(self) -> None:
        self.options.append((None, "", False))

    def populate(self, items: List[MenuItem]) -> None:
        for item in items:
            if item.item_type == "label":
                self.add_label(item.text)
            elif item.item_type == "separator":
                self.add_separator()
            elif item.item_type == "option" and item.key is not None:
                self.add_option(str(item.key), item.text)

    def show(self) -> None:
        ui.clear()
        ui.echo("\n" + "=" * 78)
        ui.echo(f"   {self.title}")
        ui.echo("=" * 78 + "\n")

        for key, text, is_selectable in self.options:
            if is_selectable:
                ui.echo(f"   {key}. {text}")
            else:
                if text:
                    ui.echo(f"  {text}")
                else:
                    ui.echo("")

        ui.echo("\n" + "=" * 78 + "\n")

    def ask(self, prompt_msg: str, error_msg: str) -> str:
        while True:
            self.show()
            choice = input(prompt_msg).strip().lower()
            if choice in self.valid_keys:
                return choice

            ui.echo(error_msg)
            input(get_string("press_enter_to_continue"))


def select_menu_action(menu_items: List[MenuItem], title_key: str) -> Optional[str]:
    menu = TerminalMenu(get_string(title_key))
    menu.populate(menu_items)

    action_map = {
        item.key: item.action for item in menu_items if item.item_type == "option"
    }

    choice = menu.ask(get_string("prompt_select"), get_string("err_invalid_selection"))
    return action_map.get(choice)
