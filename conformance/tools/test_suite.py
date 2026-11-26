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

import gzip
import io
import json
import re
import unittest
import zipfile
from collections import Counter
from pathlib import Path
from unittest import mock

import generate_cases
import generate_correction
import journal_cases
import parity_files
import parity_model
import suite

HEADER_KEYS = {
    "id", "kind", "class", "language", "requires", "summary", "variant", "code",
    "volatile", "config", "fs_before", "step", "second_volume_before", "http_files",
}
STEP_KEYS = {
    "method", "program", "program_file", "variant", "options", "params", "expect",
    "expect_at", "match", "expect_error", "validate", "fs_after", "absent", "events",
    "events_match", "second_volume_after", "setup_only",
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
    "bulk", "notable", "irreversible", "out-of-range", "link-loop", "named-params",
    "rows-changed", "events",
)
JOURNAL_TOPICS = (
    "undo-last", "redo", "undo-to", "conflict-report", "conflict-force",
    "conflict-include-later", "conflict-partial", "conflict-abort", "history",
    "no-expiry", "expiry-size", "pin-protects-from-limits", "pin-release-within-limits",
    "journal-below-reserve", "journal-op-too-large", "journal-oversize-skip",
    "journal-oversize-keep", "volume-max-size", "expiry-age-pinned",
)
FIELD_NAME = re.compile(r"^[a-z_][a-z0-9_]*$")
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


class SnapshotRules(unittest.TestCase):
    """Snapshots are checked against the rules of the case format."""

    @staticmethod
    def files(*paths: str) -> dict:
        return {"entries": [{"path": p, "type": "file", "content_utf8": ""} for p in paths]}

    def test_a_normalised_relative_path_is_accepted(self) -> None:
        self.assertEqual(suite.snapshot_errors(self.files("a.txt", "dir/sub/b.txt", "naïve/日本語.txt")), [])

    def test_paths_with_empty_dot_or_parent_segments_are_refused(self) -> None:
        for path in ("", "/abs", "a//b", "a/", "./a", "a/./b", "../a", "a/../b", "a\\b"):
            with self.subTest(path=path):
                self.assertEqual(len(suite.snapshot_errors(self.files(path))), 1)

    def test_an_entry_needs_its_content_members(self) -> None:
        both = {"entries": [{"path": "a", "type": "file", "content_utf8": "", "content_base64": ""}]}
        none = {"entries": [{"path": "a", "type": "file"}]}
        directory = {"entries": [{"path": "d", "type": "dir", "content_utf8": ""}]}
        link = {"entries": [{"path": "l", "type": "symlink"}]}
        unknown = {"entries": [{"path": "x", "type": "pipe"}]}
        for snapshot in (both, none, directory, link, unknown):
            with self.subTest(snapshot=snapshot):
                self.assertEqual(len(suite.snapshot_errors(snapshot)), 1)

    def test_unchanged_and_foreign_values(self) -> None:
        self.assertEqual(suite.snapshot_errors("unchanged"), [])
        self.assertEqual(suite.snapshot_errors("other"), ["snapshot is not an object"])
        self.assertEqual(len(suite.snapshot_errors({"format": "nurt-fs-snapshot/2", "entries": []})), 1)


class ReferenceModel(unittest.TestCase):
    """The reference model of the parity scenarios follows the order of paths of the specification."""

    def test_a_directory_comes_right_before_its_contents(self) -> None:
        self.assertEqual(
            sorted(["a-b", "a/x", "a", "a.txt"], key=parity_files.path_order),
            ["a", "a/x", "a-b", "a.txt"],
        )

    def test_found_entries_and_text_files_follow_that_order(self) -> None:
        layout = {"a/x": "1", "a-b": "2", "b": None}
        rows = parity_files.found(layout, "**", kind="any")
        self.assertEqual(
            [row["fields"]["path"]["v"] for row in rows],
            ["${base}/a", "${base}/a/x", "${base}/a-b", "${base}/b"],
        )
        self.assertEqual(parity_files.text_files(layout), ["a/x", "a-b"])

    def test_glob_patterns_stay_within_a_segment_unless_doubled(self) -> None:
        regex = parity_files.glob_regex("src/**/*.go")
        self.assertTrue(regex.match("src/a.go"))
        self.assertTrue(regex.match("src/d/e.go"))
        self.assertFalse(regex.match("src/d/e.gox"))
        self.assertFalse(parity_files.glob_regex("*.go").match("src/a.go"))
        self.assertTrue(parity_files.glob_regex("?.txt").match("b.txt"))

    def test_hidden_paths_have_a_component_starting_with_a_dot(self) -> None:
        self.assertTrue(parity_files.hidden(".git/config"))
        self.assertTrue(parity_files.hidden("src/.env"))
        self.assertFalse(parity_files.hidden("src/a.go"))

    def test_fixed_archives_do_not_depend_on_the_library(self) -> None:
        packed = parity_model.gzip_bytes(b"line\n")
        self.assertEqual(gzip.decompress(packed), b"line\n")
        self.assertEqual(packed, parity_model.gzip_bytes(b"line\n"))
        with self.assertRaises(ValueError):
            parity_model.gzip_bytes(b"x" * 0x10000)
        archive = parity_model.zip_bytes({"a.txt": "alpha\n"})
        self.assertEqual(archive, parity_model.zip_bytes({"a.txt": "alpha\n"}))
        with zipfile.ZipFile(io.BytesIO(archive)) as opened:
            self.assertEqual(opened.read("a.txt"), b"alpha\n")
            self.assertEqual(opened.getinfo("a.txt").date_time, parity_model.ZIP_TIME)

    def test_expected_snapshots_use_the_names_of_the_keyword_forms(self) -> None:
        state = parity_model.changed({"a.txt": "x"}, b_dot_txt="y", dir__c_dot_txt="z", a_dot_txt=parity_model.REMOVED)
        paths = [entry["path"] for entry in state["entries"]]
        self.assertEqual(paths, ["b.txt", "dir/c.txt"])
        directory = parity_model.tree({"d": None, "e": ""})
        self.assertEqual([e["type"] for e in directory["entries"]], ["dir", "file"])


class NameModel(unittest.TestCase):
    """The model of the name correction that the generated corpus relies on."""

    def test_edit_distance_counts_insertions_deletions_substitutions_and_transpositions(self) -> None:
        self.assertEqual(suite.osa("", ""), 0)
        self.assertEqual(suite.osa("", "abc"), 3)
        self.assertEqual(suite.osa("ab", "ba"), 1)
        self.assertEqual(suite.osa("kitten", "sitting"), 3)
        self.assertEqual(suite.osa("naïve", "naive"), 1)

    def test_thresholds_follow_the_length_of_the_name(self) -> None:
        self.assertEqual([suite.threshold(n) for n in ("ls", "cat", "find", "history", "evaluate")], [0, 0, 1, 1, 2])

    def test_names_are_written_in_the_canonical_convention(self) -> None:
        self.assertEqual(suite.snake("fooBar-baz"), "foo_bar_baz")
        self.assertEqual(suite.snake("FIND"), "find")

    def test_resolution_outcomes(self) -> None:
        names = ["find", "fire", "first"]
        self.assertEqual(suite.resolve_name("Find", names), suite.Resolution("exact", "find"))
        self.assertEqual(suite.resolve_name("fnid", ["find", "first"]).outcome, "fuzzy")
        self.assertEqual(suite.resolve_name("fine", names).outcome, "tie")
        self.assertEqual(suite.resolve_name("xyzw", ["find"]).outcome, "none")
        self.assertEqual(suite.resolve_name("gind", ["find"]).outcome, "none")
        self.assertEqual(suite.resolve_name("gind", ["find"], require_first_char=False).outcome, "fuzzy")

    def test_a_changing_command_two_edits_away_is_ambiguous(self) -> None:
        result = suite.resolve_name("evalaute", ["evaluate"], changing={"evaluate": True})
        self.assertEqual((result.outcome, result.name, result.distance), ("fuzzy", "evaluate", 1))
        result = suite.resolve_name("evlauatx", ["evaluate"], changing={"evaluate": True})
        self.assertEqual((result.outcome, result.distance), ("ambiguous", 2))

    def test_the_scanner_finds_stages_and_named_parameters(self) -> None:
        commands = {"find": None, "first": None}
        stages = suite.scan_program('find "/x" glob: "*.go" |> first 3', commands)
        self.assertEqual([(s.command, s.params) for s in stages], [("find", ["glob"]), ("first", [])])
        nested = suite.scan_program('find "a b: c" depth: 1', commands)
        self.assertEqual([(s.command, s.params) for s in nested], [("find", ["depth"])])
        with self.assertRaises(ValueError):
            suite.scan_program('find "unterminated', commands)


class CatalogConsistency(unittest.TestCase):
    """Programs, codes and reasons of the cases exist in the catalog and the specification."""

    def test_programs_use_catalog_commands_and_parameters(self) -> None:
        for case in CASES:
            for where, text in programs(case):
                with self.subTest(case=case.id, where=where):
                    for stage in suite.scan_program(text, CATALOG):
                        self.assertIn(stage.command, CATALOG)
                        cmd = CATALOG[stage.command]
                        items = [p for p in cmd.params.values() if "named_expression" in p.type]
                        for p in stage.params:
                            if p in cmd.params:
                                continue
                            self.assertTrue(items and items[0].position is not None, f"{stage.command} {p}")
                            self.assertRegex(p, FIELD_NAME)

    def test_cases_of_system_commands_name_the_system(self) -> None:
        for case in CASES:
            os_only = any(key == "code" and code == "E_OS_ONLY"
                          for key, code in suite.walk([s.get("expect") for s in case.steps()]))
            conditions = {r for r in case.data.get("requires", []) if r.startswith("os:")}
            systems = {s for _, text in programs(case) for stage in suite.scan_program(text, CATALOG)
                       for s in CATALOG[stage.command].os}
            with self.subTest(case=case.id):
                if os_only:
                    self.assertTrue(systems, "E_OS_ONLY without a command restricted to a system")
                    self.assertTrue(conditions, "E_OS_ONLY case without a system condition")
                    for system in systems:
                        self.assertEqual(conditions & suite.OS_CONDITIONS[system], set(), system)
                else:
                    for system in systems:
                        self.assertIn(f"os:{system}", conditions)

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

    def test_out_of_range_changes_nothing(self) -> None:
        for case in CASES:
            for step in case.steps():
                codes = {v for k, v in suite.walk(step.get("expect")) if k == "code"}
                if "E_OUT_OF_RANGE" in codes:
                    with self.subTest(case=case.id):
                        self.assertEqual(step.get("fs_after"), "unchanged")

    def test_inactive_journal_reasons_follow_the_contract(self) -> None:
        by_id = {c.id: c for c in CASES}
        self.assertLessEqual(set(suite.INACTIVE_IRREVERSIBLE.values()), IRREVERSIBLE)
        for case_id, inactive in journal_cases.INACTIVE_CASES.items():
            with self.subTest(case=case_id):
                self.assertIn(case_id, by_id)
                case = by_id[case_id]
                self.assertIn("config", case.data)
                report = case.steps()[0]["expect"]["report"]
                expected = suite.INACTIVE_IRREVERSIBLE[inactive]
                self.assertEqual([e["reason"] for e in report["irreversible"]], [expected])

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

    def test_cached_resolution_follows_the_content_of_the_catalog_and_not_its_identity(self) -> None:
        def catalog(category: str) -> dict[str, suite.Command]:
            command = suite.Command("rename_entries", "files", category, "reversible", category, {}, ())
            return {"rename_entries": command}

        # Every mapping gets the same identity, as a mapping that reuses the
        # identifier of one that was freed.
        with mock.patch.object(suite, "id", lambda _: 1, create=True):
            self.assertEqual(suite.resolve_command("rnme_entries", catalog("change")).outcome, "ambiguous")
            self.assertEqual(suite.resolve_command("rnme_entries", catalog("read")).outcome, "fuzzy")


def part_cases(part: str) -> list[dict]:
    """Returns the cases of the files of ``errors/`` or ``bind/``, which the executable checks cover."""
    root = suite.CONFORMANCE / part
    files = sorted(root.rglob("*.cases.toml")) + sorted(root.rglob("*.cases.json"))
    cases: list[dict] = []
    for path in files:
        data = suite.load_case_file(path)
        assert data.get("format") == suite.CASES_FORMAT, path
        assert set(data) - {"format", "case"} == set(), path
        for raw in data.get("case", []):
            cases.append(dict(raw, file=path))
    return cases


class ErrorAndBindCases(unittest.TestCase):
    """The cases of ``errors/`` and ``bind/`` follow the case format, apart from the run of the executable."""

    PARTS = {"errors": "error", "bind": "bind"}
    KEYS = HEADER_KEYS | STEP_KEYS | {"file"}

    def test_cases_have_known_keys_and_required_members(self) -> None:
        for part, kind in self.PARTS.items():
            for case in part_cases(part):
                with self.subTest(case=case.get("id")):
                    self.assertEqual(set(case) - self.KEYS, set())
                    for key in ("id", "kind", "summary"):
                        self.assertIn(key, case)
                    self.assertRegex(case["id"], suite.CASE_ID)
                    self.assertEqual(case["kind"], kind)
                    self.assertTrue(case["id"].startswith(part + "/"))
                    self.assertRegex(case.get("class", "mandatory"), suite.CLASS)
                    for requirement in case.get("requires", []):
                        self.assertRegex(requirement, suite.REQUIREMENTS)
                    steps = case.get("step", [case])
                    for step in steps:
                        self.assertFalse("program" in step and "program_file" in step)
                        self.assertIn(step.get("method", "run"), suite.METHODS)

    def test_identifiers_are_unique_across_the_whole_suite(self) -> None:
        ids = [c["id"] for part in self.PARTS for c in part_cases(part)] + [c.id for c in CASES]
        self.assertEqual([i for i, n in Counter(ids).items() if n > 1], [])

    def test_error_cases_name_codes_of_the_registry(self) -> None:
        for case in part_cases("errors"):
            with self.subTest(case=case["id"]):
                self.assertIn(case["code"], ERRORS)

    def test_snapshots_are_valid(self) -> None:
        for part in self.PARTS:
            for case in part_cases(part):
                with self.subTest(case=case["id"]):
                    if "fs_before" in case:
                        self.assertEqual(suite.snapshot_errors(case["fs_before"]), [])
                    for step in case.get("step", [case]):
                        if "fs_after" in step:
                            self.assertEqual(suite.snapshot_errors(step["fs_after"]), [])


class GoldenCases(unittest.TestCase):
    """The directories of ``golden/`` hold the files that the runner of the executable reads."""

    def directories(self) -> list[Path]:
        return sorted(path for path in (suite.CONFORMANCE / "golden").iterdir() if path.is_dir())

    def test_every_case_has_a_tree_and_a_description(self) -> None:
        # The program texts a.nurt, b.nurt and c.nurt are optional.
        for directory in self.directories():
            with self.subTest(case=directory.name):
                self.assertTrue((directory / "ast.json").is_file())
                self.assertTrue((directory / "case.toml").is_file())

    def test_trees_are_json_documents(self) -> None:
        for directory in self.directories():
            for name in ("ast.json", "c.json", "bound.json"):
                path = directory / name
                if path.is_file():
                    with self.subTest(file=f"{directory.name}/{name}"):
                        self.assertIsInstance(json.loads(path.read_text(encoding="utf-8")), dict)

    def test_hash_files_hold_one_sha256_digest(self) -> None:
        for path in sorted((suite.CONFORMANCE / "golden").rglob("*.hash")):
            with self.subTest(file=str(path.relative_to(suite.CONFORMANCE))):
                self.assertRegex(path.read_text(encoding="utf-8").strip(), r"^(sha256:)?[0-9a-f]{64}$")


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
            key = suite.coverage_key(row)
            if row["group"] == "git" or key in journal_cases.PENDING_ROWS:
                continue
            with self.subTest(row=key):
                self.assertIn(f"journal/coverage/{key}", present)

    def test_pending_journal_rows_exist_and_have_no_case(self) -> None:
        keys = {suite.coverage_key(r) for r in suite.journal_coverage() if r["group"] != "git"}
        present = set(ids("journal"))
        for key in journal_cases.PENDING_ROWS:
            with self.subTest(row=key):
                self.assertIn(key, keys)
                self.assertNotIn(f"journal/coverage/{key}", present)

    def test_journal_cases_require_the_journal(self) -> None:
        for case in CASES:
            if case.data["kind"] != "journal":
                continue
            with self.subTest(case=case.id):
                declared = case.data.get("class") == "optional:journal"
                self.assertTrue(declared or "feature:journal" in case.data.get("requires", []))

    def test_undo_restores_a_stated_tree(self) -> None:
        for case in CASES:
            if case.data["kind"] != "journal":
                continue
            for step in case.steps():
                report = step.get("expect", {}).get("report", {}) if isinstance(step.get("expect"), dict) else {}
                if report.get("kind") == "undo" and "fs_after" in step:
                    with self.subTest(case=case.id):
                        self.assertNotEqual(step["fs_after"], "unchanged")

    def test_generated_parity_and_journal_files_are_up_to_date(self) -> None:
        self.assertEqual(generate_cases.main(["--check"]), 0)

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
