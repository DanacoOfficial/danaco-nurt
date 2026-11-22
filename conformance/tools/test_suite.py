# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Checks of the execution, journal, correction and parity cases.

The checks need no implementation of the language. They validate the case
files against the case format, the command catalog and the registries of the
specification, check that the expected name corrections follow the name rules,
and check the coverage the suite promises: every command, every row of the
journal coverage table and every correction rule.

Run from the repository root:
    python3 conformance/tools/test_suite.py
"""

from __future__ import annotations

import re
import unittest
from collections import Counter
from pathlib import Path

import generate_correction
import suite

HEADER_KEYS = {
    "id", "kind", "class", "language", "requires", "summary", "variant", "code",
    "volatile", "config", "fs_before", "step",
}
STEP_KEYS = {
    "method", "program", "program_file", "variant", "options", "params", "expect",
    "expect_at", "match", "expect_error", "validate", "fs_after",
}
DIRECTORIES = {"exec": "exec", "journal": "journal", "correction": "correction", "parity": "parity"}

RULES = (
    [f"L{n:02d}" for n in range(1, 25)]
    + [f"T{n:02d}" for n in range(1, 10)]
    + [f"V{n:02d}" for n in range(1, 5)]
    + [f"B{n:02d}" for n in range(1, 41)]
    + [f"P{n:02d}" for n in range(1, 43)]
)
UNSPECIFIED_RULES = {"B21", "B26", "P20", "P28"}
"""Rules whose canonical result the specification does not fix; they have no case."""

EXEC_TOPICS = (
    "preview", "page", "fields", "stale", "utf-16", "cp852", "cp1250", "crlf",
    "bulk", "notable", "irreversible",
)
JOURNAL_TOPICS = (
    "undo-last", "redo", "undo-to", "conflict-report", "conflict-force",
    "conflict-include-later", "conflict-partial", "conflict-abort", "history",
    "no-expiry",
)
PARITY_MINIMUM = {"files": 250, "safety": 90}

CATALOG = suite.load_catalog()
ERRORS = suite.error_codes()
NOTABLE = suite.notable_codes()
IRREVERSIBLE = suite.irreversible_reasons()
CASES = suite.load_cases()


def programs(case: suite.Case) -> list[tuple[str, str]]:
    """Returns the program texts of a case in variant A that must bind: (where, text)."""
    out = []
    variant = case.data.get("variant", "a")
    for i, step in enumerate(case.steps(), 1):
        if step.get("variant", variant) != "a":
            continue
        if case.data["kind"] != "correction" and isinstance(step.get("program"), str):
            out.append((f"step {i} program", step["program"]))
        expect = step.get("expect")
        if case.data["kind"] == "correction" and isinstance(expect, dict):
            if isinstance(expect.get("program"), str):
                out.append((f"step {i} expected program", expect["program"]))
            error = expect.get("error", {})
            for c in error.get("candidates", []) if isinstance(error, dict) else []:
                if "program" in c:
                    out.append((f"step {i} candidate", c["program"]))
    return out


class CaseFormat(unittest.TestCase):
    """Every case follows the case format of the suite."""

    def test_files_have_the_collection_format(self) -> None:
        for path in suite.case_files():
            data = suite.load_case_file(path)
            self.assertEqual(data.get("format"), suite.CASES_FORMAT, path)
            self.assertEqual(set(data) - {"format", "case"}, set(), path)

    def test_cases_have_known_keys_and_required_members(self) -> None:
        for case in CASES:
            d = case.data
            with self.subTest(case=case.id or str(case.file)):
                self.assertEqual(set(d) - HEADER_KEYS - STEP_KEYS, set())
                for key in ("id", "kind", "summary"):
                    self.assertIn(key, d)
                self.assertRegex(d["id"], suite.CASE_ID)
                self.assertIn(d["kind"], suite.KINDS)
                self.assertTrue(d["id"].startswith(d["kind"] + "/"))
                top = case.file.relative_to(suite.CONFORMANCE).parts[0]
                self.assertEqual(top, DIRECTORIES[d["kind"]])
                self.assertRegex(d.get("class", "mandatory"), suite.CLASS)
                self.assertEqual(d.get("language", suite.LANGUAGE), suite.LANGUAGE)
                for r in d.get("requires", []):
                    self.assertRegex(r, suite.REQUIREMENTS)
                if "step" in d:
                    self.assertEqual(set(d) & STEP_KEYS, set())
                    for step in d["step"]:
                        self.assertEqual(set(step) - STEP_KEYS, set())
                for step in case.steps():
                    self.assertFalse("program" in step and "program_file" in step)
                    self.assertIn(step.get("method", "run"), suite.METHODS)
                    if step.get("method", "run") == "correct":
                        self.assertNotIn("options", step)

    def test_identifiers_are_unique(self) -> None:
        counts = Counter(c.id for c in CASES)
        self.assertEqual([i for i, n in counts.items() if n > 1], [])

    def test_snapshots_are_valid(self) -> None:
        for case in CASES:
            with self.subTest(case=case.id):
                if "fs_before" in case.data:
                    self.assertEqual(suite.snapshot_errors(case.data["fs_before"]), [])
                for step in case.steps():
                    if "fs_after" in step:
                        self.assertEqual(suite.snapshot_errors(step["fs_after"]), [])

    def test_configuration_files_exist(self) -> None:
        for case in CASES:
            if "config" in case.data:
                self.assertTrue((case.file.parent / case.data["config"]).is_file(), case.id)


class CatalogConsistency(unittest.TestCase):
    """Programs, codes and reasons of the cases exist in the catalog and the specification."""

    def test_programs_use_catalog_commands_and_parameters(self) -> None:
        for case in CASES:
            for where, text in programs(case):
                with self.subTest(case=case.id, where=where):
                    for stage in suite.scan_program(text, CATALOG):
                        self.assertIn(stage.command, CATALOG)
                        cmd = CATALOG[stage.command]
                        for p in stage.params:
                            self.assertIn(p, cmd.params, f"{stage.command} {p}")
                        for system in cmd.os:
                            self.assertIn(f"os:{system}", case.data.get("requires", []))

    def test_codes_and_reasons_exist(self) -> None:
        for case in CASES:
            with self.subTest(case=case.id):
                for step in case.steps():
                    expect = step.get("expect")
                    if not isinstance(expect, dict):
                        continue
                    for key, value in suite.walk(expect):
                        if key == "code" and isinstance(value, str) and value.startswith("E_"):
                            self.assertIn(value, ERRORS)
                        elif key == "code" and isinstance(value, str):
                            self.assertIn(value, NOTABLE)
                        elif key == "reason" and isinstance(value, str):
                            self.assertIn(value, IRREVERSIBLE)
                        elif key == "status" and isinstance(value, str) and case.data["kind"] != "correction":
                            self.assertIn(value, suite.STATUSES + ("available", "partial", "expired", "undone"))
                        elif key == "rule" and isinstance(value, str):
                            self.assertRegex(value, suite.RULE_ID)

    def test_report_kind_is_a_command(self) -> None:
        for case in CASES:
            for step in case.steps():
                report = step.get("expect", {}).get("report") if isinstance(step.get("expect"), dict) else None
                if isinstance(report, dict) and "kind" in report:
                    self.assertIn(report["kind"], set(CATALOG) | {"request"}, case.id)


class NameRules(unittest.TestCase):
    """Expected name corrections follow the distance, threshold and uniqueness rules."""

    def test_expected_command_corrections_follow_the_rules(self) -> None:
        for case in CASES:
            if case.data["kind"] != "correction" or "config" in case.data:
                continue
            for step in case.steps():
                expect = step.get("expect")
                if not isinstance(expect, dict):
                    continue
                for c in expect.get("corrections", []) or expect.get("report", {}).get("corrections", []):
                    if c.get("rule") == "L01" and "from" in c:
                        with self.subTest(case=case.id):
                            r = suite.resolve_command(c["from"], CATALOG)
                            self.assertEqual((r.outcome, r.name), ("fuzzy", c["to"]))

    def test_unknown_commands_are_not_correctable(self) -> None:
        for case in CASES:
            if case.data["kind"] != "correction" or "config" in case.data:
                continue
            step = case.steps()[0]
            error = step.get("expect", {}).get("error", {}) if isinstance(step.get("expect"), dict) else {}
            if error.get("code") != "E_UNKNOWN_COMMAND":
                continue
            heads = [s.command for s in suite.scan_program(step["program"], CATALOG)]
            unknown = [h for h in heads if h not in CATALOG]
            with self.subTest(case=case.id):
                self.assertTrue(unknown)
                for h in unknown:
                    self.assertIn(suite.resolve_command(h, CATALOG).outcome, ("tie", "none"))

    def test_generated_corpus_is_up_to_date(self) -> None:
        self.assertEqual(generate_correction.main(["--check"]), 0)


def ids(kind: str) -> list[str]:
    """Returns the identifiers of the cases of one kind."""
    return [c.id for c in CASES if c.data["kind"] == kind]


class Coverage(unittest.TestCase):
    """The suite covers what it promises."""

    def test_every_command_has_execution_cases(self) -> None:
        counts = Counter(i.split("/")[2] for i in ids("exec"))
        for name, cmd in CATALOG.items():
            if cmd.group == "git":
                continue
            minimum = 3 if cmd.group == "core" else 1
            with self.subTest(command=name):
                self.assertGreaterEqual(counts[name], minimum)

    def test_execution_topics_are_covered(self) -> None:
        joined = "\n".join(ids("exec"))
        for topic in EXEC_TOPICS:
            self.assertIn(topic, joined)

    def test_every_journal_coverage_row_has_an_undo_case(self) -> None:
        present = set(ids("journal"))
        for row in suite.journal_coverage():
            if row["group"] == "git":
                continue
            with self.subTest(row=suite.coverage_key(row)):
                self.assertIn(f"journal/coverage/{suite.coverage_key(row)}", present)

    def test_journal_topics_are_covered(self) -> None:
        joined = "\n".join(ids("journal"))
        for topic in JOURNAL_TOPICS:
            self.assertIn(topic, joined)

    def test_every_correction_rule_has_cases(self) -> None:
        joined = "\n".join(ids("correction"))
        missing = {r for r in RULES if not re.search(rf"/{r.lower()}-", joined)}
        self.assertEqual(missing, UNSPECIFIED_RULES)

    def test_correction_corpus_sizes(self) -> None:
        all_ids = ids("correction")
        self.assertGreaterEqual(len(all_ids), 1500)
        self.assertGreaterEqual(sum("/adversarial/" in i for i in all_ids), 400)

    def test_parity_scenarios_of_files_and_safety(self) -> None:
        counts = Counter(i.split("/")[1] for i in ids("parity"))
        for tool, minimum in PARITY_MINIMUM.items():
            with self.subTest(tool=tool):
                self.assertGreaterEqual(counts[tool], minimum)


if __name__ == "__main__":
    unittest.main()
