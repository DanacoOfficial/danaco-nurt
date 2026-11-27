# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bench/oracles/tests/test_cli.py
# Responsibility:   Script test_cli.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Tests of the command line: the standard input protocol and ``check``."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

from .support import ORACLES_DIR, entry, snap, task

TIMEOUT = 60
EXPECT = {"files": {"out.txt": {"text": "done\n"}}}


def run_module(*args: str, stdin: bytes = b"") -> subprocess.CompletedProcess[bytes]:
    """Runs ``python -m nurt_bench_oracle`` in the oracle directory."""
    return subprocess.run(
        [sys.executable, "-m", "nurt_bench_oracle", *args],
        cwd=ORACLES_DIR,
        input=stdin,
        capture_output=True,
        timeout=TIMEOUT,
        check=False,
    )


def run_input(after: dict[str, Any], observation: dict[str, Any] | None = None) -> bytes:
    """Returns the standard input of one run of the test task."""
    run = {"task": task(EXPECT), "before": snap({}), "after": after, "observation": observation or {}}
    return json.dumps(run).encode("utf-8")


class StandardInputTests(unittest.TestCase):
    def test_passed_run_prints_the_verdict_and_exits_with_zero(self) -> None:
        done = run_module(stdin=run_input(snap({"out.txt": "done\n"})))
        self.assertEqual(done.returncode, 0, done.stderr)
        verdict = json.loads(done.stdout)
        self.assertEqual(verdict["format"], "nurt-bench-verdict/1")
        self.assertTrue(verdict["passed"])
        self.assertEqual(verdict["changes"], {"out.txt": "create"})

    def test_failed_run_prints_the_verdict_and_exits_with_one(self) -> None:
        done = run_module(stdin=run_input(snap({"out.txt": "done\n", "stray": "x"})))
        self.assertEqual(done.returncode, 1)
        verdict = json.loads(done.stdout)
        self.assertTrue(verdict["completed"])
        self.assertEqual(verdict["harm"]["unexpected"], ["stray"])

    def test_accepts_snapshots_of_the_runner(self) -> None:
        after = {"exhaustive": True, "entries": [entry("out.txt", "done\n", "0644") | {"mtime": "2026-01-01T00:00:00Z"}]}
        done = run_module(stdin=run_input(after, {"answer": "", "probes": {}}))
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_rejects_input_that_is_not_a_run(self) -> None:
        for raw in (b"[]", b"{", b'{"task": {}, "before": {"entries": []}}', run_input({"entries": [{"path": "/x"}]})):
            with self.subTest(raw=raw):
                done = run_module(stdin=raw)
                self.assertEqual(done.returncode, 2)
                self.assertEqual(done.stdout, b"")
                self.assertTrue(done.stderr.startswith(b"invalid run: "), done.stderr)


class FileCommandTests(unittest.TestCase):
    def test_check_reads_the_run_from_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            files = {
                "task.json": task({"files": {"out.txt": {"text": "done\n"}}, "answer": {"contains": ["ok"]}}),
                "before.json": snap({}),
                "after.json": snap({"out.txt": "done\n"}),
            }
            for name, value in files.items():
                (base / name).write_text(json.dumps(value), encoding="utf-8")
            (base / "answer.txt").write_text("ok\n", encoding="utf-8")
            args = ["check", "--task", "task.json", "--before", "before.json", "--after", "after.json"]
            done = subprocess.run(
                [sys.executable, "-m", "nurt_bench_oracle", *args, "--answer", "answer.txt"],
                cwd=base,
                env=dict(os.environ, PYTHONPATH=str(ORACLES_DIR)),
                capture_output=True,
                timeout=TIMEOUT,
                check=False,
            )
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(json.loads(done.stdout)["passed"])

    def test_validate_accepts_the_published_task_set(self) -> None:
        done = run_module("validate")
        self.assertEqual(done.returncode, 0, done.stdout[-2000:])
        self.assertTrue(done.stdout.decode("utf-8").rstrip().endswith("tasks, 0 errors"))


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
