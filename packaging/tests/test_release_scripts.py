# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Tests of tools/release: the result table of the scenarios and the arguments of the scripts."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import support

RELEASE = support.PACKAGING.parent / "tools" / "release"
sys.path.insert(0, str(RELEASE))

import scenario_table  # noqa: E402


class ParseResultsTest(unittest.TestCase):
    def test_a_valid_file_keeps_the_order(self):
        text = "T02\tPASS\tfine\nT01\tFAIL\tbroken\nT05\tSKIP\tneeds Windows\n"
        self.assertEqual(
            scenario_table.parse_results(text),
            [("T02", "PASS", "fine"), ("T01", "FAIL", "broken"), ("T05", "SKIP", "needs Windows")],
        )

    def test_empty_lines_are_ignored(self):
        self.assertEqual(scenario_table.parse_results("\nT01\tPASS\tfine\n\n"), [("T01", "PASS", "fine")])

    def test_a_long_reason_is_shortened(self):
        results = scenario_table.parse_results("T01\tFAIL\t" + "x" * 500 + "\n")
        self.assertEqual(len(results[0][2]), scenario_table.REASON_LIMIT)
        self.assertTrue(results[0][2].endswith("..."))

    def test_malformed_input_is_rejected(self):
        for text in (
            "",
            "\n\n",
            "T01\tPASS\n",
            "T01\tPASS\tfine\textra\n",
            "T01\tMAYBE\tfine\n",
            "\tPASS\tfine\n",
            "T01\tPASS\t \n",
            "T01\tPASS\tfine\nT01\tFAIL\tagain\n",
        ):
            with self.subTest(text=text):
                with self.assertRaises(scenario_table.ResultError):
                    scenario_table.parse_results(text)


class RenderTest(unittest.TestCase):
    RESULTS = [("T01", "PASS", "a"), ("2.11", "FAIL", "b"), ("T05", "SKIP", "c")]

    def test_the_summary_counts_every_status(self):
        self.assertEqual(scenario_table.summarize(self.RESULTS), {"PASS": 1, "FAIL": 1, "SKIP": 1})

    def test_the_table_has_a_row_per_scenario_and_a_summary(self):
        lines = scenario_table.render(self.RESULTS).splitlines()
        self.assertEqual(lines[0].split(), ["Scenario", "Result", "Reason"])
        self.assertEqual([line.split()[:2] for line in lines[2:5]], [["T01", "PASS"], ["2.11", "FAIL"], ["T05", "SKIP"]])
        self.assertEqual(lines[-1], "3 scenarios: 1 PASS, 1 FAIL, 1 SKIP")

    def test_the_exit_code_is_nonzero_only_for_a_failure(self):
        self.assertEqual(scenario_table.exit_code(self.RESULTS), 1)
        self.assertEqual(scenario_table.exit_code([("T01", "PASS", "a"), ("T05", "SKIP", "c")]), 0)


class TableCommandTest(unittest.TestCase):
    def run_table(self, text):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "results.tsv"
            path.write_text(text, encoding="utf-8")
            return subprocess.run(
                [sys.executable, "-I", str(RELEASE / "scenario_table.py"), str(path)],
                capture_output=True,
                text=True,
                check=False,
            )

    def test_a_failure_gives_exit_code_1(self):
        done = self.run_table("T01\tFAIL\tbroken\n")
        self.assertEqual(done.returncode, 1)
        self.assertIn("1 FAIL", done.stdout)

    def test_success_gives_exit_code_0(self):
        self.assertEqual(self.run_table("T01\tPASS\tfine\n").returncode, 0)

    def test_malformed_input_gives_exit_code_2(self):
        done = self.run_table("T01\tbroken\n")
        self.assertEqual(done.returncode, 2)
        self.assertIn("scenario_table:", done.stderr)


class ScriptArgumentsTest(unittest.TestCase):
    def run_script(self, name, *arguments):
        return subprocess.run(
            ["bash", str(RELEASE / name), *arguments], capture_output=True, text=True, check=False
        )

    def test_the_scripts_are_valid_bash(self):
        for name in ("scenarios.sh", "build-release.sh", "clean-linux-check.sh"):
            with self.subTest(script=name):
                done = subprocess.run(["bash", "-n", str(RELEASE / name)], capture_output=True, text=True, check=False)
                self.assertEqual(done.returncode, 0, done.stderr)

    def test_missing_arguments_give_exit_code_2(self):
        for name in ("scenarios.sh", "build-release.sh", "clean-linux-check.sh"):
            with self.subTest(script=name):
                done = self.run_script(name)
                self.assertEqual(done.returncode, 2)
                self.assertIn("Usage", done.stderr)

    def test_an_invalid_version_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            done = self.run_script("build-release.sh", "1.2", str(Path(directory) / "out"))
            self.assertEqual(done.returncode, 1)
            self.assertIn("invalid version", done.stderr)

    def test_a_non_empty_output_directory_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "file").write_text("x")
            done = self.run_script("build-release.sh", "1.2.3", directory)
            self.assertEqual(done.returncode, 1)
            self.assertIn("not empty", done.stderr)

    def test_sign_requires_two_values(self):
        done = self.run_script("build-release.sh", "1.2.3", "/nonexistent/out", "--sign", "home")
        self.assertEqual(done.returncode, 2)

    def test_the_library_archive_has_no_flag_and_no_placeholder(self):
        done = self.run_script("build-release.sh", "1.2.3", "/nonexistent/out", "--lib-archive")
        self.assertEqual(done.returncode, 2)
        self.assertNotIn("--lib-archive", (RELEASE / "build-release.sh").read_text())
        self.assertNotIn("lib_pack_command", (RELEASE / "build-release.sh").read_text())

    def test_an_unknown_scenario_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            executable = Path(directory) / "nurt"
            executable.write_text("#!/bin/sh\nexit 0\n")
            executable.chmod(0o755)
            done = subprocess.run(
                ["bash", str(RELEASE / "scenarios.sh"), str(executable), str(Path(directory) / "results")],
                capture_output=True,
                text=True,
                check=False,
                env={"PATH": "/usr/bin:/bin", "TMPDIR": directory, "NURT_SCENARIO_ONLY": "T99"},
            )
            self.assertEqual(done.returncode, 2)
            self.assertIn("unknown scenario", done.stderr)

    def test_the_archive_name_is_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "nurt.zip"
            archive.write_text("x")
            done = subprocess.run(
                ["bash", str(RELEASE / "clean-linux-check.sh"), str(archive)],
                capture_output=True,
                text=True,
                check=False,
                env={"PATH": "/usr/bin:/bin", "NURT_CONTAINER_ENGINE": "true"},
            )
            self.assertEqual(done.returncode, 1)
            self.assertIn("must end in", done.stderr)


if __name__ == "__main__":
    unittest.main()
