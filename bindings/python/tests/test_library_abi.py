# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Agreement of the library backend with ``nurt.h``.

The table of possible status codes at the top of ``nurt.h`` is read from the
header; every status of every function reachable through the API is forced
in the mock library and must surface as :class:`nurt.NurtError` with the
same status name.
"""

from __future__ import annotations

import json
import re
import unittest
from typing import Any, Callable, Dict, List

import support

from nurt import Nurt, NurtError, NurtLanguageError
from nurt._errors import STATUS_CODES
from nurt._library import SIGNATURES, _diagnostics, _document, _encode, _failed, _text
from nurt._response import failure_response, query_result

COLUMNS = ("OK", "TIMEOUT", "CANCELLED", "ARG", "ABI", "IO", "INTERNAL")

CALLS: Dict[str, Callable[[Nurt], Any]] = {
    "nurt_info": lambda n: n.info(),
    "nurt_parse": lambda n: n.parse("list"),
    "nurt_print": lambda n: n.print({"nurt": "1.0", "program": "list"}),
    "nurt_correct": lambda n: n.correct("list"),
    "nurt_check": lambda n: n.check("list"),
    "nurt_run_json": lambda n: n.run("list"),
    "nurt_exec_start": lambda n: n.start("list").close(),
    "nurt_exec_wait": lambda n: _with_execution(n, "done", lambda x: x.wait()),
    "nurt_exec_cancel": lambda n: _with_execution(n, "sleep 5000", lambda x: x.cancel()),
    "nurt_page": lambda n: n.page("page-2"),
    "nurt_catalog": lambda n: n.catalog(),
    "nurt_describe": lambda n: n.describe(),
    "nurt_help": lambda n: n.help(),
    "nurt_schema": lambda n: n.schema("ast"),
    "nurt_history": lambda n: n.history(),
    "nurt_space": lambda n: n.space(),
}
"""API call that reaches each function of the table."""

NOT_REACHABLE = {
    "nurt_abi_check": "checked once when the library is loaded",
    "nurt_engine_open": "covered by test_engine_open_failure_reports_last_error",
    "nurt_engine_config_get": "not part of the binding API",
    "nurt_session_end": "not part of the binding API",
    "nurt_ext_load": "not part of the binding API",
    "nurt_ext_list": "not part of the binding API",
    "nurt_last_error": "used to report every other failure",
}


def _with_execution(nurt: Nurt, program: str, action: Callable[[Any], Any]) -> Any:
    with nurt.start(program) as execution:
        return action(execution)


def header_text() -> str:
    if not support.HEADER.is_file():
        raise unittest.SkipTest("nurt.h is not available outside the repository")
    return support.HEADER.read_text(encoding="utf-8")


def declared_functions(text: str) -> List[str]:
    return re.findall(r"NURT_API\s+[^;]*?NURT_CALL\s+(\w+)\s*\(", text)


def status_table(text: str) -> Dict[str, List[str]]:
    lines = text.splitlines()
    start = next(i for i, line in enumerate(lines) if re.search(r"\bfunction\s+OK\s+TIMEOUT\b", line))
    positions = {name: lines[start].index(name) for name in COLUMNS}
    table: Dict[str, List[str]] = {}
    for line in lines[start + 1 :]:
        match = re.match(r"\s*\*\s+(nurt_\w+)\s", line)
        if match is None:
            break
        marks = [i for i, char in enumerate(line) if char == "x" and i > match.end(1)]
        table[match.group(1)] = sorted(
            {min(COLUMNS, key=lambda name: abs(positions[name] - mark)) for mark in marks},
            key=COLUMNS.index,
        )
    return table


class HeaderAgreementTest(unittest.TestCase):
    def test_every_declared_function_has_a_signature(self) -> None:
        self.assertEqual(sorted(declared_functions(header_text())), sorted(SIGNATURES))

    def test_signatures_have_the_arity_and_void_result_of_the_header(self) -> None:
        text = header_text()
        found = 0
        for match in re.finditer(r"NURT_API\s+([^;]*?)NURT_CALL\s+(\w+)\s*\(([^;]*?)\)\s*;", text, re.S):
            result = match.group(1).rsplit("NURT_API", 1)[-1].strip()
            name, parameters = match.group(2), match.group(3).strip()
            restype, argtypes = SIGNATURES[name]
            arity = 0 if parameters in ("", "void") else parameters.count(",") + 1
            with self.subTest(function=name):
                self.assertEqual(len(argtypes), arity)
                self.assertEqual(restype is None, result == "void")
            found += 1
        self.assertEqual(found, len(SIGNATURES))

    def test_status_table_lists_known_functions(self) -> None:
        table = status_table(header_text())
        self.assertGreater(len(table), 0)
        self.assertLessEqual(set(table), set(SIGNATURES))
        self.assertEqual(set(table) - set(CALLS), set(NOT_REACHABLE))
        for function, statuses in table.items():
            with self.subTest(function=function):
                self.assertEqual(statuses[0], "OK")


class HelperTest(unittest.TestCase):
    """Conversions of the library backend that need no library."""

    def test_text_is_utf8_and_rejects_what_a_c_string_cannot_hold(self) -> None:
        self.assertIsNone(_text(None))
        self.assertEqual(_text("zażółć"), "zażółć".encode("utf-8"))
        for bad in ("a\0b", "\ud800"):
            with self.subTest(text=bad):
                with self.assertRaises(NurtError) as caught:
                    _text(bad)
                self.assertEqual(caught.exception.status, "NURT_E_ARG")
        with self.assertRaises(NurtError):
            _encode({"program": "\udfff"})
        self.assertEqual(_encode({"a": "\0"}), b'{"a":"\\u0000"}')

    def test_second_document_carries_diagnostics_and_the_language_error(self) -> None:
        self.assertEqual(_diagnostics(b'{"diagnostics":[]}', "x"), ([], None))
        error = {"code": "E_SYNTAX"}
        document = json.dumps({"diagnostics": [{"message": "m"}, 3], "error": error}).encode()
        self.assertEqual(_diagnostics(document, "x"), ([{"message": "m"}], error))
        self.assertEqual(_diagnostics(b"", "x"), ([], None))
        self.assertEqual(_diagnostics(b'[{"message":"m"}]', "x"), ([{"message": "m"}], None))
        with self.assertRaises(NurtError):
            _diagnostics(b"{", "x")

    def test_document_of_a_language_error_becomes_the_response_of_the_server(self) -> None:
        error = {"code": "E_PATH_RELATIVE", "message": "relative"}
        failed = _failed({"error": error}, "space")
        self.assertEqual(failed, {"nurt": "1.0", "report": {"nurt": "1.0", "status": "failed", "kind": "space", "error": error}})
        self.assertEqual(_failed({"volumes": []}, "space"), {"volumes": []})
        self.assertEqual(_failed({"error": error, "program": "x"}, "correct"), {"error": error, "program": "x"})
        with self.assertRaises(NurtLanguageError) as caught:
            query_result(failed)
        self.assertEqual(caught.exception.code, "E_PATH_RELATIVE")
        unknown = failure_response("catalog", {"code": "E_UNKNOWN_VALUE", "message": "no such group"})
        with self.assertRaises(NurtError) as argument:
            query_result(unknown)
        self.assertEqual(argument.exception.status, "NURT_E_ARG")
        self.assertEqual(query_result({"volumes": []}), {"volumes": []})

    def test_first_document_is_null_after_a_language_error(self) -> None:
        self.assertIsNone(_document(b"null", "x"))
        self.assertIsNone(_document(b"", "x"))
        self.assertEqual(_document(b'{"a":1}', "x"), {"a": 1})
        with self.assertRaises(NurtError) as caught:
            _document(b"[1]", "x")
        self.assertEqual(caught.exception.status, "NURT_E_INTERNAL")


class StatusMatrixTest(unittest.TestCase):
    def setUp(self) -> None:
        self.library = support.mock_library()
        self.controls = support.mock_controls(self.library)
        self.nurt = Nurt(backend="library", library=self.library)

    def tearDown(self) -> None:
        self.nurt.close()

    def test_every_possible_status_surfaces_with_its_name(self) -> None:
        for function, statuses in status_table(header_text()).items():
            call = CALLS.get(function)
            if call is None:
                continue
            call(self.nurt)
            for column in statuses[1:]:
                name = f"NURT_E_{column}"
                with self.subTest(function=function, status=name):
                    self.controls.fail_next(function, STATUS_CODES[name])
                    with self.assertRaises(NurtError) as caught:
                        call(self.nurt)
                    self.assertEqual(caught.exception.status, name)
        self.assertEqual(self.controls.live_buffers(), 0)


if __name__ == "__main__":
    unittest.main()
