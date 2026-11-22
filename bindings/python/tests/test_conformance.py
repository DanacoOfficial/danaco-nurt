# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Runner of the binding cases in ``conformance/bindings``.

A case is a JSON object::

    {"id": "...", "call": {"method": "run", "params": {...}},
     "expect": {...}, "volatile": ["/report/timing"], "error": "NURT_E_ARG"}

``params`` of ``run`` and ``check`` is a request envelope ``nurt-request/1``.
``expect`` is matched as a subset of the result after the JSON pointers in
``volatile`` are removed; ``error`` names the expected call failure. Every
case runs on both backends.
"""

from __future__ import annotations

import copy
import json
import unittest
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional

import support

from nurt import Nurt, NurtError

CASES_DIR = support.REPOSITORY_DIR / "conformance" / "bindings"


def cases() -> List[Path]:
    files = sorted(CASES_DIR.glob("*.json")) if CASES_DIR.is_dir() else []
    if not files:
        raise unittest.SkipTest("conformance/bindings has no cases")
    return files


def envelope_call(method: str) -> Callable[[Nurt, Dict[str, Any]], Any]:
    def call(nurt: Nurt, params: Dict[str, Any]) -> Any:
        request = dict(params)
        request.pop("nurt", None)
        options = dict(request.pop("options", {}) or {})
        program = request.pop("program", None)
        ast = request.pop("ast", None)
        options.update(request)
        if method == "run":
            return nurt.run(program, ast=ast, **options).raw
        return nurt.check(program, ast=ast, **options).raw

    return call


METHODS: Dict[str, Callable[[Nurt, Dict[str, Any]], Any]] = {
    "run": envelope_call("run"),
    "check": envelope_call("check"),
    "parse": lambda n, p: {"ast": n.parse(p["program"], p.get("variant", "a")).ast},
    "print": lambda n, p: {"text": n.print(p["ast"], p.get("variant", "a"))},
    "correct": lambda n, p: n.correct(p["program"], p.get("variant", "a")).raw,
    "page": lambda n, p: n.page(p["cursor"], p.get("limit")).raw,
    "describe": lambda n, p: {"text": n.describe(p.get("layer", "core"), p.get("name"), p.get("language"))},
    "help": lambda n, p: {"text": n.help(p.get("topic"))},
    "catalog": lambda n, p: n.catalog(p.get("what")),
    "schema": lambda n, p: n.schema(p["which"]),
    "history": lambda n, p: n.history(p.get("query")),
    "space": lambda n, p: n.space(p.get("volume")),
    "info": lambda n, p: n.info(),
}


def remove_pointer(document: Any, pointer: str) -> None:
    parts = [part.replace("~1", "/").replace("~0", "~") for part in pointer.split("/")[1:]]
    target = document
    for part in parts[:-1]:
        if isinstance(target, dict) and part in target:
            target = target[part]
        elif isinstance(target, list) and part.isdigit() and int(part) < len(target):
            target = target[int(part)]
        else:
            return
    if isinstance(target, dict) and parts:
        target.pop(parts[-1], None)


def mismatch(expected: Any, actual: Any, path: str = "") -> Optional[str]:
    if isinstance(expected, dict):
        if not isinstance(actual, dict):
            return f"{path}: expected an object"
        for key, value in expected.items():
            if key not in actual:
                return f"{path}/{key}: missing"
            found = mismatch(value, actual[key], f"{path}/{key}")
            if found:
                return found
        return None
    if isinstance(expected, list):
        if not isinstance(actual, list) or len(actual) != len(expected):
            return f"{path}: expected {len(expected)} items"
        for index, (left, right) in enumerate(zip(expected, actual)):
            found = mismatch(left, right, f"{path}/{index}")
            if found:
                return found
        return None
    return None if expected == actual else f"{path}: expected {expected!r}, found {actual!r}"


class ConformanceContract:
    def make(self) -> Nurt:
        raise NotImplementedError

    def test_binding_cases(self) -> None:
        with self.make() as nurt:
            for path in cases():
                case = json.loads(path.read_text("utf-8"))
                with self.subTest(case=case.get("id", path.stem)):
                    call = case["call"]
                    method = METHODS[call["method"]]
                    if "error" in case:
                        with self.assertRaises(NurtError) as caught:
                            method(nurt, call.get("params", {}))
                        self.assertEqual(caught.exception.status, case["error"])
                        continue
                    result = copy.deepcopy(method(nurt, call.get("params", {})))
                    for pointer in case.get("volatile", []):
                        remove_pointer(result, pointer)
                    self.assertIsNone(mismatch(case.get("expect", {}), result))


class ProcessConformanceTest(ConformanceContract, unittest.TestCase):
    def make(self) -> Nurt:
        return Nurt(backend="process", executable=support.mock_executable())


class LibraryConformanceTest(ConformanceContract, unittest.TestCase):
    def make(self) -> Nurt:
        return Nurt(backend="library", library=support.mock_library())


class RunnerTest(unittest.TestCase):
    def test_volatile_pointer_removes_field(self) -> None:
        document = {"report": {"timing": {"parse_ms": 1}, "status": "ok"}}
        remove_pointer(document, "/report/timing")
        remove_pointer(document, "/report/absent/field")
        self.assertEqual(document, {"report": {"status": "ok"}})

    def test_expectation_is_a_subset(self) -> None:
        actual = {"report": {"status": "ok", "kind": "find"}, "rendered": {"text": "x"}}
        self.assertIsNone(mismatch({"report": {"status": "ok"}}, actual))
        self.assertEqual(mismatch({"report": {"status": "failed"}}, actual), "/report/status: expected 'failed', found 'ok'")


if __name__ == "__main__":
    unittest.main()
