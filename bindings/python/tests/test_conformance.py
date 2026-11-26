# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Runner of the binding cases ``conformance/bindings/*.json`` (``nurt-binding-cases/1``).

The process backend runs the cases against ``nurt-conformance serve --stdio``,
which answers from the same cases. The library backend has no counterpart:
the stub of ``libnurt`` answers only recorded calls, which
``test_implementation`` replays.
"""

from __future__ import annotations

import contextlib
import json
import os
import re
import shutil
import tempfile
import unittest
from pathlib import Path
from typing import Any, Dict, Iterator, List, Mapping, Optional, Tuple

import support

from nurt import Execution, Nurt, NurtError, NurtLanguageError, Response

CASES_ROOT = support.REPOSITORY_DIR / "conformance"
CASES_DIR = CASES_ROOT / "bindings"
FORMAT = "nurt-binding-cases/1"

STEP_TIMEOUT = 30.0
"""Seconds a step waits for an execution when the case gives no timeout."""

CASE_VARIABLE = "NURT_CONFORMANCE_CASE"
CASES_VARIABLE = "NURT_CONFORMANCE_CASES"

PLACEHOLDER = re.compile(r"\$\{(base|step(\d+)(/[^}]*))\}")
RESPONSE_CALLS = frozenset({"run", "page", "wait"})
EXECUTION_CALLS = frozenset({"wait", "cancel", "events"})

Outcome = Tuple[str, Dict[str, Any], Optional[str], Optional[str]]
"""Kind of outcome, result view, status of a call error and code of a language error."""


def load_cases() -> List[Dict[str, Any]]:
    files = sorted(CASES_DIR.glob("*.json")) if CASES_DIR.is_dir() else []
    cases: List[Dict[str, Any]] = []
    for path in files:
        document = json.loads(path.read_text("utf-8"))
        if document.get("format") != FORMAT:
            raise AssertionError(f"{path.name} is not in the format {FORMAT}")
        cases.extend(document["cases"])
    if not cases:
        raise unittest.SkipTest("conformance/bindings has no cases")
    return cases


def pointer_get(document: Any, pointer: str) -> Tuple[bool, Any]:
    """Resolve a JSON pointer; returns whether it exists and its value."""
    target = document
    for raw in pointer.split("/")[1:]:
        part = raw.replace("~1", "/").replace("~0", "~")
        if isinstance(target, dict) and part in target:
            target = target[part]
        elif isinstance(target, list) and part.isdigit() and int(part) < len(target):
            target = target[int(part)]
        else:
            return False, None
    return True, target


class Placeholders:
    """Replaces ``${base}`` and ``${stepN/pointer}`` against the views of earlier steps."""

    def __init__(self, base: Optional[str]) -> None:
        self.base = base
        self.views: List[Dict[str, Any]] = []

    def substitute(self, value: Any) -> Any:
        if isinstance(value, str):
            whole = PLACEHOLDER.fullmatch(value)
            if whole is not None:
                return self._resolve(whole)
            return PLACEHOLDER.sub(lambda match: str(self._resolve(match)), value)
        if isinstance(value, list):
            return [self.substitute(item) for item in value]
        if isinstance(value, dict):
            return {key: self.substitute(item) for key, item in value.items()}
        return value

    def _resolve(self, match: "re.Match[str]") -> Any:
        if match.group(1) == "base":
            if self.base is None:
                raise AssertionError("${base} is used by a case without fs")
            return self.base
        number, pointer = int(match.group(2)), match.group(3)
        if not 1 <= number <= len(self.views):
            raise AssertionError(f"step {number} has not run")
        found, value = pointer_get(self.views[number - 1], pointer)
        if not found:
            raise AssertionError(f"the result of step {number} has no {pointer}")
        return value


def materialize(entries: List[Mapping[str, Any]], root: Path) -> None:
    """Create the files and directories of the ``fs`` member of a case."""
    for entry in entries:
        path = root / entry["path"]
        kind = entry.get("type", "file")
        path.parent.mkdir(parents=True, exist_ok=True)
        if kind == "dir":
            path.mkdir(exist_ok=True)
        elif kind == "symlink":
            os.symlink(entry["target"], path)
        else:
            path.write_bytes(str(entry.get("content_utf8", "")).encode("utf-8"))


def response_view(response: Response) -> Dict[str, Any]:
    view: Dict[str, Any] = {"ok": response.ok, "report": dict(response.report)}
    rendered = response.raw.get("rendered")
    if isinstance(rendered, dict) and isinstance(rendered.get("text"), str):
        view["text"] = rendered["text"]
    if response.error is not None:
        view["error"] = dict(response.error)
    if response.next_cursor is not None:
        view["next_cursor"] = response.next_cursor
    return view


def request_arguments(args: Mapping[str, Any]) -> Tuple[Optional[str], Any, bool, Dict[str, Any]]:
    """Split the arguments of ``run``, ``start`` and ``check`` into program, tree, flag and options."""
    arguments = dict(args)
    options = dict(arguments.pop("options", None) or {})
    raise_on_error = bool(arguments.pop("raise_on_error", False))
    program = arguments.pop("program", None)
    ast = arguments.pop("ast", None)
    options.update(arguments)
    return program, ast, raise_on_error, options


class CaseRun:
    """Calls of one case on one client."""

    def __init__(self, nurt: Nurt) -> None:
        self.nurt = nurt
        self.executions: Dict[int, Execution] = {}

    def perform(self, number: int, step: Mapping[str, Any], args: Dict[str, Any]) -> Outcome:
        call = step["call"]
        try:
            if call in EXECUTION_CALLS:
                view = getattr(self, f"call_{call}")(self.executions[step["execution"]], args)
            else:
                view = getattr(self, f"call_{call}")(number, args)
        except NurtLanguageError as exc:
            return "language_error", response_view(exc.response), None, exc.code
        except NurtError as exc:
            return "call_error", {}, exc.status, None
        if call in RESPONSE_CALLS and not view["ok"]:
            return "language_error", view, None, view.get("error", {}).get("code")
        value = view.get("value")
        for pointer in ("/error", "/report/error"):
            found, error = pointer_get(value, pointer)
            if found and isinstance(error, dict):
                return "language_error", view, None, error.get("code")
        return "ok", view, None, None

    def close(self) -> None:
        for execution in self.executions.values():
            execution.close()
        self.nurt.close()

    def call_run(self, _number: int, args: Dict[str, Any]) -> Dict[str, Any]:
        program, ast, raise_on_error, options = request_arguments(args)
        return response_view(self.nurt.run(program, ast=ast, raise_on_error=raise_on_error, **options))

    def call_start(self, number: int, args: Dict[str, Any]) -> Dict[str, Any]:
        program, ast, raise_on_error, options = request_arguments(args)
        self.executions[number] = self.nurt.start(program, ast=ast, raise_on_error=raise_on_error, **options)
        return {}

    def call_check(self, _number: int, args: Dict[str, Any]) -> Dict[str, Any]:
        program, ast, _, options = request_arguments(args)
        return {"value": self.nurt.check(program, ast=ast, **options).raw}

    def call_parse(self, _number: int, args: Dict[str, Any]) -> Dict[str, Any]:
        return {"value": self.nurt.parse(args["program"], args.get("variant", "a")).raw}

    def call_print(self, _number: int, args: Dict[str, Any]) -> Dict[str, Any]:
        return {"value": self.nurt.print(args["ast"], args.get("variant", "a"))}

    def call_correct(self, _number: int, args: Dict[str, Any]) -> Dict[str, Any]:
        return {"value": self.nurt.correct(args["program"], args.get("variant", "a")).raw}

    def call_page(self, _number: int, args: Dict[str, Any]) -> Dict[str, Any]:
        response = self.nurt.page(args["cursor"], args.get("limit"), raise_on_error=bool(args.get("raise_on_error")))
        return response_view(response)

    def call_describe(self, _number: int, args: Dict[str, Any]) -> Dict[str, Any]:
        return {"value": self.nurt.describe(args.get("layer", "core"), args.get("name"), args.get("language"))}

    def call_help(self, _number: int, args: Dict[str, Any]) -> Dict[str, Any]:
        return {"value": self.nurt.help(args.get("topic"))}

    def call_catalog(self, _number: int, args: Dict[str, Any]) -> Dict[str, Any]:
        return {"value": self.nurt.catalog(args.get("what"))}

    def call_schema(self, _number: int, args: Dict[str, Any]) -> Dict[str, Any]:
        return {"value": self.nurt.schema(args["which"])}

    def call_history(self, _number: int, args: Dict[str, Any]) -> Dict[str, Any]:
        return {"value": self.nurt.history(args.get("query"))}

    def call_space(self, _number: int, args: Dict[str, Any]) -> Dict[str, Any]:
        return {"value": self.nurt.space(args.get("volume"))}

    def call_info(self, _number: int, _args: Dict[str, Any]) -> Dict[str, Any]:
        return {"value": self.nurt.info()}

    def call_close(self, _number: int, _args: Dict[str, Any]) -> Dict[str, Any]:
        self.nurt.close()
        return {}

    def call_wait(self, execution: Execution, args: Dict[str, Any]) -> Dict[str, Any]:
        timeout = args["timeout_ms"] / 1000 if "timeout_ms" in args else STEP_TIMEOUT
        return response_view(execution.wait(timeout))

    def call_cancel(self, execution: Execution, _args: Dict[str, Any]) -> Dict[str, Any]:
        execution.cancel()
        return {}

    def call_events(self, execution: Execution, _args: Dict[str, Any]) -> Dict[str, Any]:
        # Iteration ends with the end of the events, which follows the response.
        with contextlib.suppress(NurtError, NurtLanguageError):
            execution.wait(STEP_TIMEOUT)
        return {"value": list(execution)}


def unmet_expectations(expect: Mapping[str, Any], outcome: Outcome) -> List[str]:
    kind, view, status, code = outcome
    found: List[str] = []
    if kind != expect["outcome"]:
        found.append(f"outcome {kind}, expected {expect['outcome']} (status {status}, code {code})")
    if "status" in expect and status != expect["status"]:
        found.append(f"status {status}, expected {expect['status']}")
    if "code" in expect and code != expect["code"]:
        found.append(f"code {code}, expected {expect['code']}")
    for pointer, value in expect.get("fields", {}).items():
        exists, actual = pointer_get(view, pointer)
        if not exists:
            found.append(f"{pointer} is missing")
        elif actual != value or type(actual) is not type(value):
            found.append(f"{pointer} is {actual!r}, expected {value!r}")
    for pointer in expect.get("present", []):
        if not pointer_get(view, pointer)[0]:
            found.append(f"{pointer} is missing")
    for pointer in expect.get("absent", []):
        if pointer_get(view, pointer)[0]:
            found.append(f"{pointer} is present")
    return found


def unmet_requirement(case: Mapping[str, Any], backend: str) -> Optional[str]:
    backends = case.get("backends")
    if backends and backend not in backends:
        return f"the case is not for the {backend} backend"
    for condition in case.get("requires", []):
        kind, _, value = str(condition).partition(":")
        if kind == "os" and (value == "windows") != (os.name == "nt"):
            return f"the case requires {condition}"
    return None


def run_case(nurt: Nurt, case: Mapping[str, Any]) -> List[str]:
    """Run the steps of a case; returns the unmet expectations, prefixed with the step number."""
    base_dir = tempfile.mkdtemp(prefix="nurt-binding-case-") if case.get("fs") else None
    placeholders = Placeholders(base_dir)
    run = CaseRun(nurt)
    found: List[str] = []
    try:
        if base_dir is not None:
            materialize(case["fs"], Path(base_dir))
        for number, step in enumerate(case["steps"], start=1):
            outcome = run.perform(number, step, placeholders.substitute(step.get("args", {})))
            placeholders.views.append(outcome[1])
            found.extend(f"step {number} ({step['call']}): {item}" for item in unmet_expectations(step["expect"], outcome))
    finally:
        run.close()
        if base_dir is not None:
            shutil.rmtree(base_dir, ignore_errors=True)
    return found


@contextlib.contextmanager
def environment(values: Mapping[str, str]) -> Iterator[None]:
    saved = {name: os.environ.get(name) for name in values}
    os.environ.update(values)
    try:
        yield
    finally:
        for name, value in saved.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value


class BindingCasesContract:
    backend = ""

    def make(self) -> Nurt:
        raise NotImplementedError

    def environment(self, case: Mapping[str, Any]) -> Dict[str, str]:
        return {}

    def test_binding_cases(self) -> None:
        for case in load_cases():
            with self.subTest(case=case["id"]):
                reason = unmet_requirement(case, self.backend)
                if reason is not None:
                    self.skipTest(reason)
                with environment(self.environment(case)):
                    found = run_case(self.make(), case)
                self.assertEqual(found, [])


class ProcessBindingCasesTest(BindingCasesContract, unittest.TestCase):
    backend = "process"
    server = ""

    @classmethod
    def setUpClass(cls) -> None:
        cls.server = support.conformance_server()

    def make(self) -> Nurt:
        return Nurt(backend="process", executable=self.server)

    def environment(self, case: Mapping[str, Any]) -> Dict[str, str]:
        return {CASES_VARIABLE: str(CASES_ROOT), CASE_VARIABLE: case["id"]}


class RunnerTest(unittest.TestCase):
    def test_pointer_addresses_members_and_items(self) -> None:
        document = {"value": [{"t": "exec.started"}], "a/b": {"~": 1}}
        self.assertEqual(pointer_get(document, "/value/0/t"), (True, "exec.started"))
        self.assertEqual(pointer_get(document, "/a~1b/~0"), (True, 1))
        self.assertEqual(pointer_get(document, "/value/1"), (False, None))
        self.assertEqual(pointer_get(document, ""), (True, document))

    def test_placeholders_take_values_of_earlier_steps(self) -> None:
        placeholders = Placeholders("/home/user/case")
        placeholders.views.append({"next_cursor": "c-1", "limit": 2})
        arguments = {"cursor": "${step1/next_cursor}", "limit": "${step1/limit}", "path": "${base}/src"}
        self.assertEqual(
            placeholders.substitute(arguments),
            {"cursor": "c-1", "limit": 2, "path": "/home/user/case/src"},
        )
        with self.assertRaises(AssertionError):
            placeholders.substitute("${step2/next_cursor}")

    def test_expectations_report_each_difference(self) -> None:
        outcome: Outcome = ("ok", {"ok": True, "report": {"status": "ok"}}, None, None)
        expect = {
            "outcome": "language_error",
            "code": "E_SYNTAX",
            "fields": {"/ok": 1, "/report/status": "ok"},
            "present": ["/error"],
            "absent": ["/report"],
        }
        self.assertEqual(
            unmet_expectations(expect, outcome),
            [
                "outcome ok, expected language_error (status None, code None)",
                "code None, expected E_SYNTAX",
                "/ok is True, expected 1",
                "/error is missing",
                "/report is present",
            ],
        )

    def test_request_arguments_move_envelope_fields_into_options(self) -> None:
        program, ast, raise_on_error, options = request_arguments(
            {"program": "count", "options": {"limit": 1}, "cursor": "c", "raise_on_error": True}
        )
        self.assertEqual((program, ast, raise_on_error), ("count", None, True))
        self.assertEqual(options, {"limit": 1, "cursor": "c"})


if __name__ == "__main__":
    unittest.main()
