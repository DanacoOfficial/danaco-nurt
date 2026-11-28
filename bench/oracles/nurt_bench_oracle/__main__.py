# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bench/oracles/nurt_bench_oracle/__main__.py
# Responsibility:   Script __main__.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Command line of the oracle package.

Usage::

    python3 -m nurt_bench_oracle < RUN.json
    python3 -m nurt_bench_oracle validate [--tasks DIR ...]
    python3 -m nurt_bench_oracle check --task FILE --before FILE --after FILE
        [--answer FILE] [--probes FILE]
    python3 -m nurt_bench_oracle verify --shell bash|pwsh [--tasks DIR ...]
        [--id ID ...] [--domain N ...] [--jobs N] [--timeout SECONDS] [--report FILE]

Without a command, one run is read from standard input as the JSON object
``{"task", "before", "after", "observation"}`` and the verdict is written to
standard output.
"""

from __future__ import annotations

import argparse
import json
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any, Mapping

from .oracle import check
from .reference import Outcome, print_line, report_line, verify_task, write_report
from .snapshot import SnapshotError
from .tasks import (
    FIXTURES_DIR,
    SHELL_OF_PLATFORM,
    TASKS_DIR,
    load_schema,
    load_tasks,
    set_errors,
    strip_private,
    task_errors,
)

DEFAULT_TIMEOUT = 20.0
DEFAULT_JOBS = 4
EXIT_PASSED = 0
EXIT_FAILED = 1
EXIT_INVALID_INPUT = 2
RUN_MEMBERS = ("task", "before", "after")


def read_json(path: str) -> Any:
    """Reads a JSON file."""
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def write_verdict(verdict: Mapping[str, Any], indent: int | None) -> int:
    """Writes a verdict to standard output and returns the exit code."""
    sys.stdout.write(json.dumps(verdict, indent=indent, sort_keys=True, ensure_ascii=False) + "\n")
    return EXIT_PASSED if verdict["passed"] else EXIT_FAILED


def judge_run(run: Any) -> dict[str, Any]:
    """Evaluates one run given as ``{"task", "before", "after", "observation"}``.

    # Errors
    Raises :class:`ValueError` when the object lacks a member or a snapshot
    is malformed.
    """
    if not isinstance(run, Mapping):
        raise ValueError("input must be a JSON object")
    missing = [name for name in RUN_MEMBERS if not isinstance(run.get(name), Mapping)]
    if missing:
        raise ValueError(f"input lacks the object members {', '.join(missing)}")
    observation = run.get("observation") or {}
    if not isinstance(observation, Mapping):
        raise ValueError("observation must be an object")
    task = run["task"]
    if not isinstance(task.get("expect"), Mapping):
        raise ValueError("task has no expect object")
    return check(run["before"], run["after"], task, observation)


def command_stdin() -> int:
    """Evaluates the run read from standard input and prints the verdict."""
    try:
        run = json.loads(sys.stdin.buffer.read().decode("utf-8"))
        verdict = judge_run(run)
    except (ValueError, KeyError, TypeError, SnapshotError) as error:
        sys.stderr.write(f"invalid run: {error}\n")
        return EXIT_INVALID_INPUT
    return write_verdict(verdict, None)


def command_validate(args: argparse.Namespace) -> int:
    """Validates the task set and prints the errors."""
    tasks = load_tasks(Path(directory) for directory in args.tasks)
    validator = load_schema()
    errors = set_errors(tasks)
    for task in tasks:
        errors.extend(f"{task.get('id')}: {error}" for error in task_errors(task, validator, FIXTURES_DIR))
    for error in errors:
        print_line(error)
    print_line(f"{len(tasks)} tasks, {len(errors)} errors")
    return EXIT_FAILED if errors or not tasks else EXIT_PASSED


def command_check(args: argparse.Namespace) -> int:
    """Evaluates one run given as files and prints the verdict."""
    observation: dict[str, Any] = {}
    if args.answer:
        observation["answer"] = Path(args.answer).read_text(encoding="utf-8")
    if args.probes:
        observation["probes"] = read_json(args.probes)
    verdict = check(read_json(args.before), read_json(args.after), read_json(args.task), observation)
    return write_verdict(verdict, 2)


def command_verify(args: argparse.Namespace) -> int:
    """Runs the reference and defective solutions of the selected tasks."""
    tasks = [
        strip_private(task)
        for task in load_tasks(Path(directory) for directory in args.tasks)
        if args.shell in SHELL_OF_PLATFORM[task["platform"]]
        and (not args.id or task["id"] in args.id)
        and (not args.domain or task["domain"] in args.domain)
    ]

    def run(task: dict[str, Any]) -> tuple[str, list[Outcome]]:
        outcomes = verify_task(task, args.shell, args.timeout)
        print_line(report_line(task["id"], outcomes))
        return task["id"], outcomes

    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        results = dict(pool.map(run, tasks))
    failed = sorted(task_id for task_id, outcomes in results.items() if not all(o.ok for o in outcomes))
    runs = sum(len(outcomes) for outcomes in results.values())
    print_line(f"{len(results)} tasks, {runs} runs, {len(failed)} tasks with failures")
    if args.report:
        write_report(Path(args.report), results)
    return EXIT_FAILED if failed or not results else EXIT_PASSED


def parser() -> argparse.ArgumentParser:
    """Builds the argument parser."""
    root = argparse.ArgumentParser(
        prog="nurt_bench_oracle",
        description="Without a command, read one run from standard input and print its verdict.",
    )
    commands = root.add_subparsers(dest="command")
    validate = commands.add_parser("validate", help="validate the task set")
    validate.add_argument("--tasks", action="append", default=None, help="task directory")
    validate.set_defaults(handler=command_validate)
    single = commands.add_parser("check", help="evaluate one run given as files")
    single.add_argument("--task", required=True)
    single.add_argument("--before", required=True)
    single.add_argument("--after", required=True)
    single.add_argument("--answer")
    single.add_argument("--probes")
    single.set_defaults(handler=command_check)
    verify = commands.add_parser("verify", help="test the oracles on reference solutions")
    verify.add_argument("--shell", choices=("bash", "pwsh"), required=True)
    verify.add_argument("--tasks", action="append", default=None, help="task directory")
    verify.add_argument("--id", action="append", default=[], help="task identifier")
    verify.add_argument("--domain", action="append", type=int, default=[], help="domain number")
    verify.add_argument("--jobs", type=int, default=DEFAULT_JOBS)
    verify.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT)
    verify.add_argument("--report")
    verify.set_defaults(handler=command_verify)
    return root


def main(argv: list[str] | None = None) -> int:
    """Runs the command line and returns the exit code."""
    args = parser().parse_args(argv)
    if args.command is None:
        return command_stdin()
    if getattr(args, "tasks", None) is None:
        args.tasks = [str(TASKS_DIR)]
    return int(args.handler(args))


if __name__ == "__main__":
    sys.exit(main())

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
