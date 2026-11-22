# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Command line of the oracle package.

Usage::

    python3 -m nurt_bench_oracle validate [--tasks DIR ...]
    python3 -m nurt_bench_oracle check --task FILE --before FILE --after FILE
        [--answer FILE] [--probes FILE]
    python3 -m nurt_bench_oracle verify --shell bash|pwsh [--tasks DIR ...]
        [--id ID ...] [--domain N ...] [--jobs N] [--timeout SECONDS] [--report FILE]
"""

from __future__ import annotations

import argparse
import json
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any

from .oracle import check
from .reference import Outcome, print_line, report_line, verify_task, write_report
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


def read_json(path: str) -> Any:
    """Reads a JSON file."""
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


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
    return 1 if errors or not tasks else 0


def command_check(args: argparse.Namespace) -> int:
    """Evaluates one run and prints the verdict."""
    observation: dict[str, Any] = {}
    if args.answer:
        observation["answer"] = Path(args.answer).read_text(encoding="utf-8")
    if args.probes:
        observation["probes"] = read_json(args.probes)
    verdict = check(read_json(args.before), read_json(args.after), read_json(args.task), observation)
    sys.stdout.write(json.dumps(verdict, indent=2, sort_keys=True) + "\n")
    return 0 if verdict["passed"] else 1


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
    return 1 if failed or not results else 0


def parser() -> argparse.ArgumentParser:
    """Builds the argument parser."""
    root = argparse.ArgumentParser(prog="nurt_bench_oracle")
    commands = root.add_subparsers(dest="command", required=True)
    validate = commands.add_parser("validate", help="validate the task set")
    validate.add_argument("--tasks", action="append", default=None, help="task directory")
    validate.set_defaults(handler=command_validate)
    single = commands.add_parser("check", help="evaluate one run")
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
    if getattr(args, "tasks", None) is None:
        args.tasks = [str(TASKS_DIR)]
    return int(args.handler(args))


if __name__ == "__main__":
    sys.exit(main())
