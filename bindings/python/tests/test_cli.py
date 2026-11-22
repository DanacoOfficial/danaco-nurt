# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Console script ``nurt`` and ``python -m nurt``."""

from __future__ import annotations

import os
import subprocess
import sys
import unittest
from typing import Dict, List

import support

EXIT_USAGE = 2
EXIT_NOT_FOUND = 127


def run_console(arguments: List[str], executable: str) -> "subprocess.CompletedProcess[str]":
    environment: Dict[str, str] = dict(os.environ)
    environment["NURT_EXECUTABLE"] = executable
    environment["PYTHONPATH"] = str(support.PACKAGE_DIR / "src")
    return subprocess.run(
        [sys.executable, "-m", "nurt", *arguments],
        capture_output=True,
        text=True,
        env=environment,
        timeout=60,
    )


class ConsoleScriptTest(unittest.TestCase):
    def test_runs_located_executable_with_arguments(self) -> None:
        result = run_console(["--version"], support.mock_executable())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "nurt 0.1.0")

    def test_passes_exit_code_through(self) -> None:
        result = run_console(["unknown-command"], support.mock_executable())
        self.assertEqual(result.returncode, EXIT_USAGE)
        self.assertIn("usage", result.stderr)

    def test_missing_executable_exits_with_127(self) -> None:
        missing = str(support.scratch() / "absent" / "nurt")
        result = run_console(["--version"], missing)
        self.assertEqual(result.returncode, EXIT_NOT_FOUND)
        self.assertTrue(result.stderr.startswith("nurt: nurt executable not found"), result.stderr)
        self.assertIn(missing, result.stderr)


if __name__ == "__main__":
    unittest.main()
