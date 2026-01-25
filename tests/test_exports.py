import ast
from pathlib import Path
from typing import Optional

import pytest


def _module_name_for_path(file_path: Path, package_root: Path) -> str:
    rel_path = file_path.relative_to(package_root)
    parts = rel_path.with_suffix("").parts
    if parts[-1] == "__init__":
        parts = parts[:-1]
    return ".".join(("ltbox",) + parts)


def _resolve_imported_module(
    current_module: str, node: ast.ImportFrom, alias_name: str
) -> Optional[str]:
    if node.module is None:
        base_module = current_module.rsplit(".", node.level)[0]
    else:
        if node.level:
            base_module = current_module.rsplit(".", node.level)[0]
            base_module = f"{base_module}.{node.module}" if base_module else node.module
        else:
            base_module = node.module
    if not base_module:
        return None
    return f"{base_module}.{alias_name}"


def _collect_attribute_accesses(
    tree: ast.AST, aliases: dict[str, str]
) -> dict[str, set[str]]:
    attr_map: dict[str, set[str]] = {mod: set() for mod in aliases.values()}
    for node in ast.walk(tree):
        if isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name):
            alias = node.value.id
            if alias in aliases:
                attr_map[aliases[alias]].add(node.attr)
    return attr_map


def _find_exports_to_validate(package_root: Path) -> dict[str, set[str]]:
    package_modules = {
        _module_name_for_path(init_path, package_root)
        for init_path in package_root.rglob("__init__.py")
    }
    exports: dict[str, set[str]] = {module: set() for module in package_modules}
    for py_file in package_root.rglob("*.py"):
        tree = ast.parse(py_file.read_text(encoding="utf-8"))
        current_module = _module_name_for_path(py_file, package_root)
        alias_map: dict[str, str] = {}
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                for alias in node.names:
                    if alias.name in package_modules:
                        alias_map[alias.asname or alias.name.rsplit(".", 1)[-1]] = (
                            alias.name
                        )
            elif isinstance(node, ast.ImportFrom):
                for alias in node.names:
                    resolved = _resolve_imported_module(
                        current_module, node, alias.name
                    )
                    if resolved and resolved in package_modules:
                        alias_map[alias.asname or alias.name] = resolved
        attr_map = _collect_attribute_accesses(tree, alias_map)
        for module_name, attrs in attr_map.items():
            exports[module_name].update(attrs)
    return exports


def test_init_exports_cover_attribute_accesses():
    package_root = Path(__file__).resolve().parents[1] / "bin" / "ltbox"
    exports = _find_exports_to_validate(package_root)
    missing = {}
    for module_name, attrs in exports.items():
        if not attrs:
            continue
        module = pytest.importorskip(module_name)
        missing_attrs = sorted(attr for attr in attrs if not hasattr(module, attr))
        if missing_attrs:
            missing[module_name] = missing_attrs
    assert not missing, f"Missing exports: {missing}"
