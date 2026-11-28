# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bench/oracles/tests/test_reference.py
# Responsibility:   Script test_reference.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Tests of the execution of reference and defective solutions."""

from __future__ import annotations

import os
import shutil
import sys
import unittest
from pathlib import Path
from typing import Any
from unittest import mock

from nurt_bench_oracle import reference
from nurt_bench_oracle.reference import Execution, Outcome, report_line, verify_task
from nurt_bench_oracle.snapshot import SnapshotError, as_index

from .support import snap

TIMEOUT = 60.0
HAS_BASH = os.name == "posix" and shutil.which("bash") is not None
HAS_PWSH = shutil.which("pwsh") is not None
SORTED = "alpha\nbeta\n"


def sort_task(**fields: Any) -> dict[str, Any]:
    """Returns a task that sorts ``names.txt`` and keeps ``keep.txt``."""
    data: dict[str, Any] = {
        "id": "test-001",
        "platform": "cross",
        "fs_before": snap({"names.txt": "beta\nalpha\n", "keep.txt": "keep\n"}),
        "expect": {"files": {"names.txt": {"text": SORTED}}},
        "forbidden": [],
        "reference_bash": "sort -o names.txt names.txt\n",
        "reference_pwsh": (
            "$lines = Get-Content names.txt | Sort-Object\n"
            "[IO.File]::WriteAllText((Join-Path $PWD 'names.txt'), ($lines -join \"`n\") + \"`n\")\n"
        ),
    }
    data.update(fields)
    return data


def labels(outcomes: list[Outcome]) -> list[str]:
    """Returns the labels of outcomes that did not decide as expected."""
    return [outcome.label for outcome in outcomes if not outcome.ok]


class ReportTests(unittest.TestCase):
    def test_report_line_names_the_failed_solutions(self) -> None:
        good = Outcome("reference", True, {"passed": True})
        bad = Outcome("empty solution", False, {"passed": True})
        self.assertEqual(report_line("t-001", [good]), "ok t-001 (1 runs)")
        self.assertEqual(report_line("t-001", [good, bad]), "FAIL t-001 (2 runs): empty solution")
        self.assertFalse(Outcome("reference", True, None, "setup failed").ok)

    def test_task_without_reference_for_the_shell_has_no_outcomes(self) -> None:
        self.assertEqual(verify_task(sort_task(reference_bash=None), "bash", TIMEOUT), [])


class ExecutionFailureTests(unittest.TestCase):
    """Failures of the execution are outcomes, not passes and not crashes."""

    def test_a_reference_that_reached_the_time_limit_is_not_accepted(self) -> None:
        task = sort_task()
        before = as_index(task["fs_before"])
        after = as_index(snap({"names.txt": SORTED, "keep.txt": "keep\n"}))
        late = Execution(before, after, "", {}, timed_out=True)
        with mock.patch("nurt_bench_oracle.reference.execute", return_value=late):
            outcomes = verify_task(task, "bash", 5.0)
        self.assertEqual(len(outcomes), 1)
        self.assertEqual(outcomes[0].label, "reference")
        self.assertFalse(outcomes[0].ok)
        self.assertIn("did not end within 5 s", outcomes[0].error or "")

    def test_a_malformed_snapshot_is_reported_as_an_error_of_the_solution(self) -> None:
        with mock.patch("nurt_bench_oracle.reference.execute", side_effect=SnapshotError("bad entry")):
            outcomes = verify_task(sort_task(), "bash", 5.0)
        self.assertEqual([(o.label, o.error) for o in outcomes], [("reference", "bad entry")])
        self.assertFalse(outcomes[0].ok)

    def test_a_defective_solution_that_fails_to_run_is_an_error_outcome(self) -> None:
        task = sort_task()
        before = as_index(task["fs_before"])
        after = as_index(snap({"names.txt": SORTED, "keep.txt": "keep\n"}))
        done = Execution(before, after, "", {}, timed_out=False)
        failure = RuntimeError("setup failed")
        with mock.patch("nurt_bench_oracle.reference.execute", side_effect=[done, failure, failure]):
            outcomes = verify_task(task, "bash", 5.0)
        self.assertEqual(
            [o.label for o in outcomes],
            ["reference", "empty solution", "reference and deletion of keep.txt"],
        )
        self.assertEqual([o.error for o in outcomes[1:]], ["setup failed", "setup failed"])
        self.assertFalse(outcomes[1].ok)


@unittest.skipUnless(HAS_BASH, "bash is not available")
class BashTests(unittest.TestCase):
    def test_reference_passes_and_defects_are_rejected(self) -> None:
        negative = {"shell": "bash", "script": "sort names.txt > names.txt\n", "defect": "truncates the input"}
        outcomes = verify_task(sort_task(negatives=[negative]), "bash", TIMEOUT)
        self.assertEqual(
            [outcome.label for outcome in outcomes],
            ["reference", "empty solution", "reference and deletion of keep.txt", "truncates the input"],
        )
        self.assertEqual(labels(outcomes), [])

    def test_deletion_applies_from_the_task_root(self) -> None:
        moved = sort_task(
            fs_before=snap({"sub/names.txt": "beta\nalpha\n", "keep.txt": "keep\n"}),
            expect={"files": {"sub/names.txt": {"text": SORTED}}},
            reference_bash="cd sub && sort -o names.txt names.txt\nexit 3\n",
        )
        outcomes = verify_task(moved, "bash", TIMEOUT)
        self.assertEqual(labels(outcomes), [])
        self.assertIn("reference and deletion of keep.txt", [outcome.label for outcome in outcomes])

    def test_failing_setup_is_reported_as_an_error(self) -> None:
        outcomes = verify_task(sort_task(setup={"description": "fails", "bash": "exit 3"}), "bash", TIMEOUT)
        self.assertEqual(len(outcomes), 1)
        self.assertEqual(outcomes[0].error, "setup failed with exit code 3")

    def test_probes_and_answer_are_observed(self) -> None:
        probe = {"id": "count", "kind": "command",
                 "argv": [sys.executable, "-c", "print(open('names.txt').read().count(chr(10)))"]}
        counted = sort_task(
            probes=[probe],
            expect={"files": {"names.txt": {"text": SORTED}}, "probes": {"count": {"stdout": "2", "exit": 0}},
                    "answer": {"contains": ["sorted"]}},
            reference_bash="sort -o names.txt names.txt\necho sorted\n",
        )
        outcomes = verify_task(counted, "bash", TIMEOUT)
        self.assertEqual(labels(outcomes), [])
        self.assertEqual(outcomes[0].verdict["failures"] if outcomes[0].verdict else None, [])

    def test_time_limit_stops_the_solution(self) -> None:
        slow = sort_task(reference_bash="sleep 30\n")
        outcomes = verify_task(slow, "bash", 1.0)
        self.assertEqual(outcomes[0].label, "reference")
        self.assertFalse(outcomes[0].ok)


@unittest.skipUnless(HAS_PWSH, "PowerShell 7 is not available")
class PowerShellTests(unittest.TestCase):
    def test_reference_passes_and_defects_are_rejected(self) -> None:
        outcomes = verify_task(sort_task(), "pwsh", TIMEOUT)
        self.assertEqual(
            [outcome.label for outcome in outcomes],
            ["reference", "empty solution", "reference and deletion of keep.txt"],
        )
        self.assertEqual(labels(outcomes), [])


class ProcessTableTests(unittest.TestCase):
    """The process table of systems without ``/proc`` is read with ps and lsof."""

    def setUp(self) -> None:
        patcher = mock.patch.object(reference, "PROC", Path("/nonexistent-proc"))
        patcher.start()
        self.addCleanup(patcher.stop)

    @staticmethod
    def answer(stdout: str, code: int = 0) -> mock.Mock:
        return mock.Mock(returncode=code, stdout=stdout, stderr="")

    def test_commands_are_read_with_ps_when_there_is_no_proc(self) -> None:
        listing = "  101 /bin/sleep 30\n  202 bash -c echo marker-4711\nnot a process line\n"
        with mock.patch.object(reference.subprocess, "run", return_value=self.answer(listing)) as run:
            commands = reference.process_commands()
        self.assertEqual(commands, {101: "/bin/sleep 30", 202: "bash -c echo marker-4711"})
        self.assertEqual(run.call_args.args[0][0], "ps")

    @unittest.skipUnless(os.name == "posix", "POSIX systems only")
    def test_processes_are_counted_by_their_command_line_without_proc(self) -> None:
        listing = f"  {os.getpid()} python marker-4711\n  202 bash -c marker-4711\n  303 other\n"
        with mock.patch.object(reference.subprocess, "run", return_value=self.answer(listing)):
            self.assertEqual(reference.count_processes("marker-4711"), 1)

    def test_working_directories_are_read_with_lsof_when_there_is_no_proc(self) -> None:
        listing = "p101\nn/tmp/task/sub\np202\nn/home/user\n"
        with mock.patch.object(reference.subprocess, "run", return_value=self.answer(listing)) as run:
            directories = reference.process_directories()
        self.assertEqual(directories, {101: "/tmp/task/sub", 202: "/home/user"})
        self.assertEqual(run.call_args.args[0][0], "lsof")

    @unittest.skipUnless(os.name == "posix", "POSIX systems only")
    def test_leftovers_below_the_task_directory_are_stopped_without_proc(self) -> None:
        listing = "p101\nn/tmp/task/sub\np202\nn/home/user\np303\nn/tmp/taskother\n"
        with mock.patch.object(reference.subprocess, "run", return_value=self.answer(listing)):
            with mock.patch.object(reference.os, "kill") as kill:
                reference.stop_leftovers(Path("/tmp/task"))
        kill.assert_called_once_with(101, reference.signal.SIGKILL)


class WindowsProbeTests(unittest.TestCase):
    def test_a_probe_that_fails_is_an_error_and_not_an_empty_result(self) -> None:
        failed = mock.Mock(returncode=1, stdout="", stderr="pwsh: access denied")
        with mock.patch.object(reference.subprocess, "run", return_value=failed):
            with self.assertRaisesRegex(RuntimeError, "access denied"):
                reference.windows_probe({"kind": "registry", "key": "HKCU\\Software\\X", "value": "v"})

    def test_a_process_probe_on_windows_checks_the_exit_code(self) -> None:
        failed = mock.Mock(returncode=2, stdout="", stderr="no cim")
        with mock.patch.object(reference.os, "name", "nt"):
            with mock.patch.object(reference.subprocess, "run", return_value=failed):
                with self.assertRaisesRegex(RuntimeError, "no cim"):
                    reference.count_processes("anything")

    def test_stopping_a_process_on_windows_ends_its_tree(self) -> None:
        process = mock.Mock(pid=4321)
        with mock.patch.object(reference.subprocess, "run") as run:
            reference.stop_process(process, posix=False)
        self.assertEqual(run.call_args.args[0], ["taskkill", "/PID", "4321", "/T", "/F"])
        process.kill.assert_called_once_with()


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
