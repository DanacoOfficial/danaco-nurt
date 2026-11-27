# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bindings/python/tests/test_types.py
# Responsibility:   Script test_types.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Agreement of ``nurt.types`` with the JSON schemas and the golden files of ``spec``."""

from __future__ import annotations

import importlib.util
import json
import re
import sys
import typing
import unittest
from pathlib import Path
from typing import Any, List

import support

from nurt import __version__, types

SCHEMAS = support.SPEC_DIR / "schemas"
GOLDEN = support.SPEC_DIR / "golden"


def load_generator() -> Any:
    path = support.PACKAGE_DIR / "tools" / "gen_types.py"
    spec = importlib.util.spec_from_file_location("gen_types", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules["gen_types"] = module
    spec.loader.exec_module(module)
    return module


def is_typed_dict(kind: Any) -> bool:
    return isinstance(kind, type) and hasattr(kind, "__required_keys__")


def problems(value: Any, kind: Any, path: str = "") -> List[str]:
    """Differences between a JSON value and a type of ``nurt.types``."""
    origin = typing.get_origin(kind)
    arguments = typing.get_args(kind)
    if kind is Any:
        return []
    if is_typed_dict(kind):
        if not isinstance(value, dict):
            return [f"{path}: expected an object"]
        hints = typing.get_type_hints(kind)
        found = [f"{path}/{key}: unknown field" for key in value if key not in hints]
        found += [f"{path}/{key}: missing field" for key in kind.__required_keys__ if key not in value]
        for key, item in value.items():
            if key in hints:
                found += problems(item, hints[key], f"{path}/{key}")
        return found
    if origin is typing.Literal:
        return [] if value in arguments else [f"{path}: {value!r} not in {arguments}"]
    if origin is typing.Union:
        options = [problems(value, option, path) for option in arguments]
        return [] if any(not found for found in options) else options[0]
    if origin is list:
        if not isinstance(value, list):
            return [f"{path}: expected an array"]
        return [p for i, item in enumerate(value) for p in problems(item, arguments[0], f"{path}/{i}")]
    if origin is dict:
        if not isinstance(value, dict):
            return [f"{path}: expected an object"]
        return [p for key, item in value.items() for p in problems(item, arguments[1], f"{path}/{key}")]
    if kind is type(None):
        return [] if value is None else [f"{path}: expected null"]
    if kind is float:
        return [] if isinstance(value, (int, float)) and not isinstance(value, bool) else [f"{path}: expected a number"]
    if kind is int and isinstance(value, bool):
        return [f"{path}: expected an integer"]
    return [] if isinstance(value, kind) else [f"{path}: expected {kind.__name__}"]


def golden(directory: str) -> List[Path]:
    files = sorted((GOLDEN / directory).glob("*.json"))
    if not files:
        raise unittest.SkipTest(f"golden files of {directory} are not available")
    return files


class TypesTest(unittest.TestCase):
    def test_module_matches_schemas(self) -> None:
        if not SCHEMAS.is_dir():
            self.skipTest("spec/schemas is not available outside the repository")
        generator = load_generator()
        _, current = generator.strip_header((support.PACKAGE_DIR / "src" / "nurt" / "types.py").read_text("utf-8"))
        self.assertEqual(current, generator.render(SCHEMAS), "run tools/gen_types.py")

    def test_golden_reports_conform_to_report_type(self) -> None:
        for path in golden("report"):
            with self.subTest(file=path.name):
                self.assertEqual(problems(json.loads(path.read_text("utf-8")), types.Report), [])

    def test_golden_requests_conform_to_request_type(self) -> None:
        for path in golden("request"):
            with self.subTest(file=path.name):
                self.assertEqual(problems(json.loads(path.read_text("utf-8")), types.Request), [])

    def test_checker_reports_differences(self) -> None:
        report = {"nurt": "1.0", "status": "unknown", "kind": "find", "colour": 1}
        self.assertEqual(
            problems(report, types.Report),
            ["/colour: unknown field", "/status: 'unknown' not in ('ok', 'planned', 'running', "
             "'partial', 'rolled_back', 'failed')"],
        )

    def test_package_version_matches_workspace(self) -> None:
        manifest = support.REPOSITORY_DIR / "Cargo.toml"
        if not manifest.is_file():
            self.skipTest("Cargo.toml is not available outside the repository")
        match = re.search(r'^\[workspace\.package\][^\[]*?^version = "([^"]+)"', manifest.read_text("utf-8"), re.M | re.S)
        assert match is not None
        self.assertEqual(__version__, match.group(1))


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
