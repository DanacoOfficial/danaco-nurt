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

import re
import unittest
from typing import Any, Callable, Dict, List

import support

from nurt import Nurt, NurtError
from nurt._errors import STATUS_CODES
from nurt._library import SIGNATURES

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

    def test_status_table_lists_known_functions(self) -> None:
        table = status_table(header_text())
        self.assertGreater(len(table), 0)
        self.assertLessEqual(set(table), set(SIGNATURES))
        self.assertEqual(set(table) - set(CALLS), set(NOT_REACHABLE))
        for function, statuses in table.items():
            with self.subTest(function=function):
                self.assertEqual(statuses[0], "OK")


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
