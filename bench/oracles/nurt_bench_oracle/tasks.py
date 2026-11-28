# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bench/oracles/nurt_bench_oracle/tasks.py
# Responsibility:   Script tasks.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Loading and validation of the task set."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any, Iterable, Mapping

from .paths import matches
from .schema import Validator
from .snapshot import SnapshotError, resolve_fs_before

BENCH_DIR = Path(__file__).resolve().parents[2]
TASKS_DIR = BENCH_DIR / "tasks"
FIXTURES_DIR = BENCH_DIR / "fixtures"
SCHEMA_PATH = TASKS_DIR / "schema" / "nurt-bench-task-1.schema.json"
HOLDOUT_LIST = TASKS_DIR / "holdout.json"
SCRIPTS_DIR = Path(__file__).resolve().parent / "scripts"

DOMAINS: dict[int, tuple[str, str]] = {
    1: ("01-search", "search"),
    2: ("02-multi-edit", "multi-edit"),
    3: ("03-single-edit", "single-edit"),
    4: ("04-structured", "structured"),
    5: ("05-code", "code"),
    6: ("06-programs", "programs"),
    7: ("07-background", "background"),
    8: ("08-system", "system"),
    9: ("09-archives", "archives"),
    10: ("10-git", "git"),
    11: ("11-admin", "admin"),
    12: ("12-provoke", "provoke"),
}
TASKS_PER_DOMAIN = 60
PLATFORM_SHARE = {"linux": 30, "windows": 15, "cross": 15}
HOLDOUT_PER_DOMAIN = 6
SHELL_OF_PLATFORM = {"linux": ("bash",), "macos": ("bash",), "windows": ("pwsh",), "cross": ("bash", "pwsh")}


def canonical_hash(task: Mapping[str, Any]) -> str:
    """Returns the SHA-256 of a task serialized in canonical JSON."""
    text = json.dumps(task, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
    return hashlib.sha256(text.encode("ascii")).hexdigest()


def task_files(directories: Iterable[Path]) -> list[Path]:
    """Returns the task files below the given directories, sorted."""
    found: list[Path] = []
    for directory in directories:
        for domain_dir, _ in DOMAINS.values():
            found.extend(sorted((directory / domain_dir).glob("*.json")))
    return found


def load_tasks(directories: Iterable[Path]) -> list[dict[str, Any]]:
    """Loads all tasks below the given directories."""
    tasks = []
    for path in task_files(directories):
        with path.open(encoding="utf-8") as handle:
            task = json.load(handle)
        task["_file"] = str(path)
        tasks.append(task)
    return tasks


def strip_private(task: Mapping[str, Any]) -> dict[str, Any]:
    """Returns the task without loader fields."""
    return {key: value for key, value in task.items() if not key.startswith("_")}


def load_schema() -> Validator:
    """Loads the task schema."""
    with SCHEMA_PATH.open(encoding="utf-8") as handle:
        return Validator(json.load(handle))


def task_errors(task: Mapping[str, Any], validator: Validator, fixtures_dir: Path) -> list[str]:
    """Returns the schema and consistency errors of one task."""
    data = strip_private(task)
    errors = validator.errors(data)
    if errors:
        return errors
    domain_dir, slug = DOMAINS[data["domain"]]
    if not data["id"].startswith(f"{slug}-"):
        errors.append(f"id must start with {slug}-")
    file = task.get("_file")
    if file and (Path(file).parent.name != domain_dir or Path(file).stem != data["id"]):
        errors.append(f"file must be {domain_dir}/{data['id']}.json")
    try:
        resolve_fs_before(data["fs_before"], fixtures_dir)
    except (SnapshotError, ValueError) as error:
        errors.append(f"fs_before: {error}")
    probe_ids = [probe["id"] for probe in data.get("probes", [])]
    if len(probe_ids) != len(set(probe_ids)):
        errors.append("probe identifiers are not unique")
    expectations = [data["expect"], *data["expect"].get("alternatives", [])]
    for expectation in expectations:
        for probe_id in expectation.get("probes", {}):
            if probe_id not in probe_ids:
                errors.append(f"expect.probes names unknown probe {probe_id}")
        for path in expectation.get("files", {}):
            for rule in data["forbidden"]:
                if matches(path, rule["path"]) and rule["change"] == "any":
                    errors.append(f"expected path {path} is forbidden by {rule['path']}")
    if not any(
        expectation.get(key) for expectation in expectations for key in ("files", "answer", "probes")
    ):
        errors.append("expect has no condition")
    if "oracle" in data:
        script = SCRIPTS_DIR / f"{data['oracle']['script']}.py"
        if not script.is_file():
            errors.append(f"oracle script {script.name} not found")
    for negative in data.get("negatives", []):
        if negative["shell"] not in SHELL_OF_PLATFORM[data["platform"]]:
            errors.append(f"negative for {negative['shell']} on platform {data['platform']}")
    return errors


def set_errors(tasks: list[Mapping[str, Any]]) -> list[str]:
    """Returns the errors of the task set as a whole: uniqueness of fields."""
    errors: list[str] = []
    seen_ids: dict[str, str] = {}
    seen_nl: dict[str, str] = {}
    for task in tasks:
        task_id = task.get("id", "")
        if task_id in seen_ids:
            errors.append(f"{task_id}: duplicate id")
        seen_ids[task_id] = task_id
        text = " ".join(str(task.get("nl", "")).split()).lower()
        if text in seen_nl:
            errors.append(f"{task_id}: same description as {seen_nl[text]}")
        seen_nl[text] = task_id
    return errors

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
