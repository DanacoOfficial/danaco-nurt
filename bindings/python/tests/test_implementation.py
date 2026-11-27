# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bindings/python/tests/test_implementation.py
# Responsibility:   Script test_implementation.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Tests against the real artifacts: the stub build of ``libnurt`` and ``nurt serve``.

The library tests replay the recorded cases of the stub
(``crates/nurt-ffi/tests/fixtures/stub``) through the public API. The stub
answers a call only when its input equals the recorded one, so the cases are
copied into a temporary directory with the request options that the client
adds (the default language). The process tests exercise the methods that
the engine behind ``nurt serve`` answers independently of the language front end.
"""

from __future__ import annotations

import json
import os
import shutil
import tempfile
import threading
import unittest
from pathlib import Path
from typing import Any, Dict, List, Optional

import support

from nurt import (
    Nurt,
    NurtError,
    NurtLanguageError,
)

STUB_CASES = support.REPOSITORY_DIR / "crates" / "nurt-ffi" / "tests" / "fixtures" / "stub"
STUB_DIR_VARIABLE = "NURT_STUB_DIR"
LANGUAGE = {"language": "en"}
FIND_PROGRAM = 'find "/srv/app" glob: "*.go"'
THREADS = 8
CALLS_PER_THREAD = 25
EVENT_TIMEOUT = 30.0


def stub_cases(target: Path) -> None:
    """Copy the recorded cases with the request options of the client added."""
    target.mkdir(parents=True, exist_ok=True)
    for source in sorted(STUB_CASES.glob("*.json")):
        case = json.loads(source.read_text(encoding="utf-8"))
        function = case["function"]
        if function in ("run", "check"):
            case["input"] = {**case["input"], "options": dict(LANGUAGE)}
        elif function == "describe":
            case["input"] = {"layer": "core", **LANGUAGE}
        (target / source.name).write_text(json.dumps(case), encoding="utf-8")


class LibraryStubTest(unittest.TestCase):
    """The library backend on the stub build of ``libnurt``."""

    library = ""
    directory = ""
    saved: Optional[str] = None

    @classmethod
    def setUpClass(cls) -> None:
        cls.library = support.implementation_library()
        cls.directory = tempfile.mkdtemp(prefix="nurt-stub-cases-")
        stub_cases(Path(cls.directory))
        cls.saved = os.environ.get(STUB_DIR_VARIABLE)
        os.environ[STUB_DIR_VARIABLE] = cls.directory

    @classmethod
    def tearDownClass(cls) -> None:
        if cls.saved is None:
            os.environ.pop(STUB_DIR_VARIABLE, None)
        else:
            os.environ[STUB_DIR_VARIABLE] = cls.saved
        shutil.rmtree(cls.directory, ignore_errors=True)

    def open(self) -> Nurt:
        nurt = Nurt(backend="library", library=self.library)
        self.addCleanup(nurt.close)
        return nurt

    def test_info_reports_the_interface_version(self) -> None:
        info = self.open().info()
        self.assertEqual(info["abi"]["major"], 1)
        self.assertIn("ast", info["formats"])

    def test_run_returns_the_recorded_response_with_a_cursor(self) -> None:
        response = self.open().run(FIND_PROGRAM)
        self.assertTrue(response.ok)
        self.assertEqual(response.report["kind"], "find")
        self.assertEqual(response.next_cursor, "c_91fe")

    def test_language_error_is_returned_and_raised_on_request(self) -> None:
        nurt = self.open()
        program = 'find "/srv/app" glob: ["*.go"'
        response = nurt.run(program)
        self.assertFalse(response.ok)
        self.assertEqual((response.error or {}).get("code"), "E_SYNTAX")
        with self.assertRaises(NurtLanguageError) as caught:
            nurt.run(program, raise_on_error=True)
        self.assertEqual(caught.exception.code, "E_SYNTAX")

    def test_call_without_recorded_case_is_internal_error(self) -> None:
        with self.assertRaises(NurtError) as caught:
            self.open().run('find "/srv/unrecorded"')
        self.assertEqual(caught.exception.status, "NURT_E_INTERNAL")

    def test_panic_inside_the_engine_is_internal_error(self) -> None:
        with self.assertRaises(NurtError) as caught:
            self.open().run('find "/srv/panic"')
        self.assertEqual(caught.exception.status, "NURT_E_INTERNAL")

    def test_events_precede_the_response_in_order(self) -> None:
        nurt = self.open()
        with nurt.start('find "/srv/app" glob: "*.rs"') as execution:
            response = execution.wait(EVENT_TIMEOUT)
            events = list(execution)
        self.assertTrue(response.ok)
        kinds = [event["t"] for event in events]
        self.assertEqual(kinds[0], "exec.started")
        self.assertEqual(kinds[-1], "exec.finished")
        self.assertEqual([kind for kind in kinds if kind in ("log", "progress")], ["log", "progress", "progress"])
        self.assertEqual([event["seq"] for event in events], sorted(event["seq"] for event in events))

    def test_cancel_ends_a_slow_execution(self) -> None:
        nurt = self.open()
        with nurt.start('find "/srv/archive"') as execution:
            execution.cancel()
            response = execution.wait(EVENT_TIMEOUT)
        self.assertFalse(response.ok)
        self.assertEqual((response.error or {}).get("code"), "E_CANCELLED")

    def test_parse_print_and_correct(self) -> None:
        nurt = self.open()
        parsed = nurt.parse('find "/srv/app"')
        self.assertTrue(parsed.ok)
        self.assertEqual(nurt.print(parsed.ast), nurt.print(parsed.ast))
        self.assertEqual(len(nurt.correct('fnd "/srv/app"').corrections), 1)

    def test_check_returns_the_bound_tree_without_diagnostics(self) -> None:
        result = self.open().check(FIND_PROGRAM)
        self.assertTrue(result.ok)
        self.assertEqual(result.raw["effect"], "read")

    def test_page_follows_the_recorded_cursor(self) -> None:
        self.assertTrue(self.open().page("c_91fe").ok)

    def test_text_and_catalog_queries(self) -> None:
        nurt = self.open()
        self.assertEqual(nurt.describe(), "Programs are pipelines of commands joined by |>.")
        self.assertEqual(nurt.help(), "Topics: commands, lists, paths.")
        self.assertEqual(nurt.catalog()["commands"][0]["name"], "find")
        self.assertEqual(nurt.history(), {"operations": []})
        self.assertEqual(nurt.space(), {"volumes": []})

    def test_schema_is_served_without_a_case(self) -> None:
        nurt = self.open()
        self.assertEqual(nurt.schema("tool")["title"], "nurt-tool/1")
        with self.assertRaises(NurtError) as caught:
            nurt.schema("unknown")
        self.assertEqual(caught.exception.status, "NURT_E_ARG")

    def test_threads_share_one_client(self) -> None:
        nurt = self.open()
        failures: List[BaseException] = []

        def work() -> None:
            try:
                for _ in range(CALLS_PER_THREAD):
                    if nurt.run(FIND_PROGRAM).next_cursor != "c_91fe":
                        raise AssertionError("unexpected response")
            except BaseException as exc:  # noqa: BLE001 - reported to the test thread
                failures.append(exc)

        threads = [threading.Thread(target=work) for _ in range(THREADS)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        self.assertEqual(failures, [])


class ServeProcessTest(unittest.TestCase):
    """The process backend on ``nurt serve --stdio``."""

    executable = ""

    @classmethod
    def setUpClass(cls) -> None:
        cls.executable = support.implementation_executable()

    def open(self) -> Nurt:
        nurt = Nurt(backend="process", executable=self.executable)
        self.addCleanup(nurt.close)
        return nurt

    def test_info_describes_the_engine(self) -> None:
        info = self.open().info()
        self.assertEqual(info["formats"]["ast"], "1.0")
        self.assertIsInstance(info["version"], str)

    def test_catalog_lists_the_command_groups(self) -> None:
        catalog = self.open().catalog()
        self.assertEqual(catalog["format"], "nurt-catalog-result/1")
        self.assertGreater(catalog["total"], 0)
        self.assertTrue(catalog["groups"])

    def test_schema_returns_the_published_document(self) -> None:
        self.assertEqual(self.open().schema("tool")["title"], "nurt-tool/1")

    def test_unknown_schema_is_argument_error(self) -> None:
        with self.assertRaises(NurtError) as caught:
            self.open().schema("unknown")
        self.assertEqual(caught.exception.status, "NURT_E_ARG")

    def test_malformed_cursor_is_a_language_error_in_the_response(self) -> None:
        nurt = self.open()
        response = nurt.page("malformed")
        self.assertFalse(response.ok)
        self.assertEqual((response.error or {}).get("code"), "E_ARG")
        with self.assertRaises(NurtLanguageError):
            nurt.page("malformed", raise_on_error=True)

    def test_run_returns_a_response_with_a_status(self) -> None:
        response = self.open().run('find "." glob: "*.rs"')
        self.assertIn(response.report["status"], ("ok", "failed"))
        self.assertEqual(response.ok, response.error is None)

    def test_events_of_an_execution_carry_the_request(self) -> None:
        nurt = self.open()
        with nurt.start('find "." glob: "*.rs"') as execution:
            execution.wait(EVENT_TIMEOUT)
            events = list(execution)
        kinds = [event["t"] for event in events]
        self.assertEqual(kinds[0], "exec.started")
        self.assertEqual(kinds[-1], "exec.finished")
        self.assertEqual(len({event["request"] for event in events}), 1)

    def test_language_queries_return_text_or_a_language_error(self) -> None:
        nurt = self.open()
        for query in (nurt.describe, nurt.help):
            try:
                self.assertIsInstance(query(), str)
            except NurtLanguageError as error:
                self.assertTrue(error.code.startswith("E_"))

    def test_parse_check_and_correct_return_results(self) -> None:
        nurt = self.open()
        self.assertIsInstance(nurt.parse('find "."').ok, bool)
        self.assertIsInstance(nurt.check('find "."').ok, bool)
        self.assertIsInstance(nurt.correct('fnd "."').corrections, list)

    def test_threads_share_one_process(self) -> None:
        nurt = self.open()
        failures: List[BaseException] = []
        versions: List[Dict[str, Any]] = []

        def work() -> None:
            try:
                for _ in range(CALLS_PER_THREAD):
                    versions.append(nurt.info()["formats"])
            except BaseException as exc:  # noqa: BLE001 - reported to the test thread
                failures.append(exc)

        threads = [threading.Thread(target=work) for _ in range(THREADS)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        self.assertEqual(failures, [])
        self.assertEqual(len(versions), THREADS * CALLS_PER_THREAD)
        self.assertEqual(len({json.dumps(item, sort_keys=True) for item in versions}), 1)

    def test_process_exit_is_reported_and_the_next_call_restarts_it(self) -> None:
        nurt = self.open()
        nurt.info()
        connection = nurt._backend._connection  # noqa: SLF001 - the test ends the child process
        connection.process.kill()
        connection.process.wait()
        try:
            nurt.info()
        except NurtError:
            pass
        self.assertIn("formats", nurt.info())


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
