# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Search for the ``nurt`` executable and the ``libnurt`` library."""

from __future__ import annotations

import os
import stat
import tempfile
import unittest
from pathlib import Path

import support  # noqa: F401  (puts the package on the import path)

from nurt import NurtError
from nurt._locate import (
    executable_names,
    find_executable,
    find_library,
    is_console_launcher,
    library_names,
)

NAME = executable_names()[0]


def touch(path: Path, content: str = "") -> str:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return str(path)


class LocateTest(unittest.TestCase):
    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)
        self.empty_bundle = self.root / "bundle-empty"

    def tearDown(self) -> None:
        self._directory.cleanup()

    def test_argument_takes_precedence_over_variable(self) -> None:
        argument = touch(self.root / "argument" / NAME)
        variable = touch(self.root / "variable" / NAME)
        found = find_executable(argument, environ={"NURT_EXECUTABLE": variable}, bundle_dir=self.empty_bundle)
        self.assertEqual(found, argument)

    def test_missing_argument_is_an_error_without_fallback(self) -> None:
        variable = touch(self.root / "variable" / NAME)
        missing = str(self.root / "missing" / NAME)
        with self.assertRaises(NurtError) as caught:
            find_executable(missing, environ={"NURT_EXECUTABLE": variable}, bundle_dir=self.empty_bundle)
        self.assertEqual(caught.exception.status, "NURT_E_IO")
        self.assertIn(missing, caught.exception.message)
        self.assertNotIn(variable, caught.exception.message)

    def test_variable_is_used_without_argument(self) -> None:
        variable = touch(self.root / "variable" / NAME)
        self.assertEqual(find_executable(environ={"NURT_EXECUTABLE": variable}, bundle_dir=self.empty_bundle), variable)

    def test_bundled_file_precedes_path(self) -> None:
        bundled = touch(self.root / "bundle" / NAME)
        on_path = touch(self.root / "bin" / NAME)
        found = find_executable(environ={"PATH": str(Path(on_path).parent)}, bundle_dir=self.root / "bundle")
        self.assertEqual(found, bundled)

    @unittest.skipIf(os.name == "nt", "console scripts on Windows are executables")
    def test_path_search_skips_console_script_of_the_package(self) -> None:
        launcher = touch(self.root / "scripts" / NAME, "#!/usr/bin/python3\nfrom nurt._cli import main\nmain()\n")
        program = touch(self.root / "bin" / NAME, "#!/bin/sh\n")
        search = os.pathsep.join([str(Path(launcher).parent), str(Path(program).parent)])
        self.assertTrue(is_console_launcher(launcher))
        self.assertFalse(is_console_launcher(program))
        self.assertEqual(find_executable(environ={"PATH": search}, bundle_dir=self.empty_bundle), program)

    def test_nothing_found_lists_checked_locations(self) -> None:
        with self.assertRaises(NurtError) as caught:
            find_executable(environ={"PATH": str(self.root / "nothing")}, bundle_dir=self.empty_bundle)
        message = caught.exception.message
        self.assertIn("bundled", message)
        self.assertIn("PATH", message)
        self.assertIn("NURT_EXECUTABLE", message)
        self.assertEqual(len(caught.exception.detail["checked"]), 2)

    def test_library_is_not_searched_on_path(self) -> None:
        touch(self.root / "bin" / library_names()[0])
        with self.assertRaises(NurtError) as caught:
            find_library(environ={"PATH": str(self.root / "bin")}, bundle_dir=self.empty_bundle)
        self.assertIn("NURT_LIBRARY", caught.exception.message)

    def test_library_variable_and_bundle(self) -> None:
        bundled = touch(self.root / "bundle" / library_names()[0])
        variable = touch(self.root / "variable" / library_names()[0])
        self.assertEqual(find_library(environ={"NURT_LIBRARY": variable}, bundle_dir=self.root / "bundle"), variable)
        self.assertEqual(find_library(environ={}, bundle_dir=self.root / "bundle"), bundled)

    def test_file_names_per_system(self) -> None:
        self.assertEqual(executable_names("win32"), ("nurt.exe",))
        self.assertEqual(executable_names("linux"), ("nurt",))
        self.assertEqual(library_names("win32"), ("nurt.dll",))
        self.assertEqual(library_names("darwin")[0], "libnurt.1.dylib")
        self.assertEqual(library_names("linux")[0], "libnurt.so.1")


if __name__ == "__main__":
    unittest.main()
