# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bench/oracles/tests/test_taskset.py
# Responsibility:   Script test_taskset.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Tests of the published task set, its distribution and the held-out list.

The held-out tasks are checked when the environment variable
``NURT_BENCH_HOLDOUT`` names a directory with their domain directories.
"""

from __future__ import annotations

import collections
import json
import os
import re
import unittest
from pathlib import Path
from typing import Any

from nurt_bench_oracle.snapshot import index_from_snapshot
from nurt_bench_oracle.tasks import (
    DOMAINS,
    FIXTURES_DIR,
    HOLDOUT_LIST,
    HOLDOUT_PER_DOMAIN,
    PLATFORM_SHARE,
    TASKS_DIR,
    TASKS_PER_DOMAIN,
    canonical_hash,
    load_schema,
    load_tasks,
    set_errors,
    strip_private,
    task_errors,
)

HOLDOUT_ENV = "NURT_BENCH_HOLDOUT"
MIN_TASKS_PER_PROVOCATION = 4
PROVOKE_DOMAIN = 12
URL = re.compile(r"https?://([A-Za-z0-9.-]+)")
PERMITTED_HOSTS = re.compile(r"(?:[a-z0-9-]+\.)*example(?:\.com)?\.?|localhost|127\.0\.0\.1")


def provocation_classes() -> list[str]:
    """Returns the provocation classes of the task schema."""
    return list(load_schema().root["properties"]["provocation"]["enum"])


def text_fields(task: dict[str, Any]) -> list[str]:
    """Returns the free text fields of a task that are shown or executed."""
    fields = [task["nl"], task["oracle_program"], task.get("reference_bash", ""), task.get("reference_pwsh", "")]
    fields.extend(task.get("setup", {}).values())
    return fields


class PublishedSetTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.tasks = load_tasks([TASKS_DIR])
        with HOLDOUT_LIST.open(encoding="utf-8") as handle:
            cls.holdout = json.load(handle)

    def test_every_task_is_valid(self) -> None:
        validator = load_schema()
        errors = set_errors(self.tasks)
        for task in self.tasks:
            errors.extend(f"{task['id']}: {error}" for error in task_errors(task, validator, FIXTURES_DIR))
        self.assertEqual(errors, [])

    def test_published_tasks_are_public(self) -> None:
        self.assertEqual({task["split"] for task in self.tasks}, {"public"})
        self.assertEqual(self.holdout["format"], "nurt-bench-holdout/1")

    def test_each_domain_has_sixty_tasks_with_six_held_out(self) -> None:
        public = collections.Counter(task["domain"] for task in self.tasks)
        held = collections.Counter(item["domain"] for item in self.holdout["tasks"])
        for domain in DOMAINS:
            with self.subTest(domain=domain):
                self.assertEqual(held[domain], HOLDOUT_PER_DOMAIN)
                self.assertEqual(public[domain] + held[domain], TASKS_PER_DOMAIN)

    def test_platform_shares_per_domain(self) -> None:
        counts = collections.Counter((task["domain"], task["platform"]) for task in self.tasks)
        counts.update((item["domain"], item["platform"]) for item in self.holdout["tasks"])
        for domain in DOMAINS:
            with self.subTest(domain=domain):
                self.assertEqual({p: counts[(domain, p)] for p in PLATFORM_SHARE}, PLATFORM_SHARE)

    def test_identifiers_cover_each_domain_without_gaps(self) -> None:
        ids = [task["id"] for task in self.tasks] + [item["id"] for item in self.holdout["tasks"]]
        self.assertEqual(len(ids), len(set(ids)))
        for domain, (_, slug) in DOMAINS.items():
            expected = {f"{slug}-{number:03d}" for number in range(1, TASKS_PER_DOMAIN + 1)}
            self.assertEqual({i for i in ids if i.rsplit("-", 1)[0] == slug}, expected)

    def test_held_out_hashes_are_well_formed(self) -> None:
        for item in self.holdout["tasks"]:
            self.assertRegex(item["sha256"], r"^[0-9a-f]{64}$")

    def test_provocation_classes_are_all_present(self) -> None:
        present = {task["provocation"] for task in self.tasks if task["domain"] == PROVOKE_DOMAIN}
        self.assertEqual(present, set(provocation_classes()))

    def test_tasks_name_only_example_hosts(self) -> None:
        for task in self.tasks:
            for text in text_fields(task):
                for host in URL.findall(text):
                    with self.subTest(task=task["id"], host=host):
                        self.assertIsNotNone(PERMITTED_HOSTS.fullmatch(host), host)

    def test_fixtures_are_valid_snapshots(self) -> None:
        fixtures = sorted(FIXTURES_DIR.glob("*.json"))
        self.assertTrue(fixtures)
        for path in fixtures:
            with self.subTest(fixture=path.stem), path.open(encoding="utf-8") as handle:
                self.assertTrue(index_from_snapshot(json.load(handle)))


@unittest.skipUnless(os.environ.get(HOLDOUT_ENV), f"{HOLDOUT_ENV} is not set")
class HeldOutSetTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.tasks = load_tasks([Path(os.environ[HOLDOUT_ENV])])
        with HOLDOUT_LIST.open(encoding="utf-8") as handle:
            cls.listed = {item["id"]: item for item in json.load(handle)["tasks"]}

    def test_held_out_tasks_match_the_published_list(self) -> None:
        self.assertEqual({task["id"] for task in self.tasks}, set(self.listed))
        for task in self.tasks:
            with self.subTest(task=task["id"]):
                item = self.listed[task["id"]]
                self.assertEqual(task["split"], "holdout")
                self.assertEqual((task["domain"], task["platform"]), (item["domain"], item["platform"]))
                self.assertEqual(canonical_hash(strip_private(task)), item["sha256"])

    def test_held_out_tasks_are_valid(self) -> None:
        validator = load_schema()
        public = load_tasks([TASKS_DIR])
        errors = set_errors(public + self.tasks)
        for task in self.tasks:
            errors.extend(f"{task['id']}: {error}" for error in task_errors(task, validator, FIXTURES_DIR))
        self.assertEqual(errors, [])

    def test_each_provocation_class_has_four_tasks(self) -> None:
        tasks = load_tasks([TASKS_DIR]) + self.tasks
        counts = collections.Counter(task["provocation"] for task in tasks if task["domain"] == PROVOKE_DOMAIN)
        for name in provocation_classes():
            with self.subTest(provocation=name):
                self.assertGreaterEqual(counts[name], MIN_TASKS_PER_PROVOCATION)


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
