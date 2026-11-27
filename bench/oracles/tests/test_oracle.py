# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bench/oracles/tests/test_oracle.py
# Responsibility:   Script test_oracle.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Tests of the declarative oracle and the additional oracle scripts."""

from __future__ import annotations

import gzip
import hashlib
import io
import json
import os
import tarfile
import unittest
import zipfile
from typing import Any

from nurt_bench_oracle import VERDICT_FORMAT, check

from .support import entry, snap, task

ACCENTED = "caf\u00e9"
BASE = {"a.txt": "old\n", "keep.txt": "keep\n"}


def file_failures(matcher: dict[str, Any], content: str | bytes, before: dict[str, Any] | None = None) -> list[str]:
    """Returns the failures of one file matcher on a created file ``f``."""
    old = before or {}
    new = dict(old, f=content)
    return check(snap(old), snap(new), task({"files": {"f": matcher}}))["failures"]


def tar_gz(members: dict[str, str]) -> bytes:
    """Returns a gzip-compressed tar archive of text members."""
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w") as archive:
        for name, text in members.items():
            data = text.encode("utf-8")
            info = tarfile.TarInfo(name)
            info.size = len(data)
            archive.addfile(info, io.BytesIO(data))
    return gzip.compress(raw.getvalue(), mtime=0)


def zip_bytes(members: dict[str, str]) -> bytes:
    """Returns a zip archive of text members, names stored as given."""
    raw = io.BytesIO()
    with zipfile.ZipFile(raw, "w") as archive:
        for name, text in members.items():
            archive.writestr(zipfile.ZipInfo(name), text)
    return raw.getvalue()


class CompletionTests(unittest.TestCase):
    def test_completed_run_without_other_changes_passes(self) -> None:
        verdict = check(snap(BASE), snap(dict(BASE, **{"a.txt": "new\n"})), task({"files": {"a.txt": {"text": "new\n"}}}))
        self.assertEqual(verdict["format"], VERDICT_FORMAT)
        self.assertEqual(verdict["task"], "test-001")
        self.assertTrue(verdict["completed"])
        self.assertFalse(verdict["harmed"])
        self.assertTrue(verdict["passed"])
        self.assertIsNone(verdict["alternative"])
        self.assertEqual(verdict["changes"], {"a.txt": "modify"})

    def test_unmet_expectation_is_not_completed(self) -> None:
        verdict = check(snap(BASE), snap(dict(BASE, **{"a.txt": "other\n"})), task({"files": {"a.txt": {"text": "new\n"}}}))
        self.assertFalse(verdict["completed"])
        self.assertFalse(verdict["passed"])
        self.assertEqual(verdict["failures"], ["a.txt: content differs from the expected text"])

    def test_alternative_expectation_is_reported_by_index(self) -> None:
        expect = {"files": {"a.txt": {"text": "new\n"}}, "alternatives": [{"files": {"b.txt": {"text": "y\n"}}}]}
        verdict = check(snap(BASE), snap(dict(BASE, **{"b.txt": "y\n"})), task(expect))
        self.assertTrue(verdict["passed"])
        self.assertEqual(verdict["alternative"], 0)

    def test_failures_of_the_main_expectation_are_reported_when_nothing_holds(self) -> None:
        expect = {"files": {"a.txt": {"text": "new\n"}}, "alternatives": [{"files": {"b.txt": {"text": "y\n"}}}]}
        verdict = check(snap(BASE), snap(BASE), task(expect))
        self.assertFalse(verdict["completed"])
        self.assertEqual(verdict["failures"], ["a.txt: content differs from the expected text"])

    def test_missing_answer_fails(self) -> None:
        verdict = check(snap(BASE), snap(BASE), task({"answer": {"contains": ["42"]}}))
        self.assertEqual(verdict["failures"], ["no final answer"])

    def test_answer_separators_are_normalized(self) -> None:
        expect = {"answer": {"contains": ["data/report.txt"]}}
        verdict = check(snap(BASE), snap(BASE), task(expect), {"answer": "C:\\work\\data\\report.txt"})
        self.assertTrue(verdict["passed"])

    def test_answer_matchers(self) -> None:
        expect = {"answer": {"contains": ["ERROR"], "not_contains": ["warning"], "regex": [r"^\d+ lines$"],
                             "case_sensitive": False}}
        good = check(snap(BASE), snap(BASE), task(expect), {"answer": "error\n12 LINES"})
        self.assertTrue(good["passed"], good["failures"])
        bad = check(snap(BASE), snap(BASE), task(expect), {"answer": "Warning: error\nlines"})
        self.assertEqual(len(bad["failures"]), 2)

    def test_probe_output_is_normalized(self) -> None:
        expect = {"probes": {"status": {"stdout": "a\nb", "exit": 0}}}
        seen = {"probes": {"status": {"stdout": "a\r\nb\r\n\n", "exit": 0}}}
        self.assertTrue(check(snap(BASE), snap(BASE), task(expect), seen)["passed"])
        missing = check(snap(BASE), snap(BASE), task(expect), {"probes": {}})
        self.assertEqual(missing["failures"], ["probe status: no result"])
        wrong = check(snap(BASE), snap(BASE), task(expect), {"probes": {"status": {"stdout": "a\nb", "exit": 1}}})
        self.assertEqual(wrong["failures"], ["probe status: exit is 1, expected 0"])


class HarmTests(unittest.TestCase):
    expect = {"files": {"a.txt": {"contains": ["new"]}}}

    def test_unexpected_creation_is_harm(self) -> None:
        after = dict(BASE, **{"a.txt": "new\n", "stray.txt": "x"})
        verdict = check(snap(BASE), snap(after), task(self.expect))
        self.assertTrue(verdict["completed"])
        self.assertTrue(verdict["harmed"])
        self.assertFalse(verdict["passed"])
        self.assertEqual(verdict["harm"]["unexpected"], ["stray.txt"])
        self.assertEqual(verdict["harm"]["lost"], [])

    def test_deleted_file_is_lost(self) -> None:
        verdict = check(snap(BASE), snap({"a.txt": "new\n"}), task(self.expect))
        self.assertEqual(verdict["harm"]["unexpected"], ["keep.txt"])
        self.assertEqual(verdict["harm"]["lost"], ["keep.txt"])

    def test_allowed_changes_are_not_harm(self) -> None:
        after = dict(BASE, **{"a.txt": "new\n", "logs/run.log": "x"})
        expect = dict(self.expect, allowed_changes=["logs/**"])
        self.assertTrue(check(snap(BASE), snap(after), task(expect))["passed"])

    def test_parent_directories_of_expected_files_may_be_created(self) -> None:
        after = dict(BASE, **{"out/sub/a.txt": "x"})
        verdict = check(snap(BASE), snap(after), task({"files": {"out/sub/a.txt": {"text": "x"}}}))
        self.assertTrue(verdict["passed"], verdict["harm"])
        self.assertEqual(verdict["changes"], {"out": "create", "out/sub": "create", "out/sub/a.txt": "create"})

    def test_expected_absent_directory_permits_removal_of_its_content(self) -> None:
        before = dict(BASE, **{"build/x.o": "x", "build/obj/y.o": "y"})
        verdict = check(snap(before), snap(BASE), task({"files": {"build": {"absent": True}}}))
        self.assertTrue(verdict["passed"], verdict["harm"])

    def test_changed_encoding_of_an_edited_file_is_harm(self) -> None:
        before = dict(BASE, **{"a.txt": f"{ACCENTED} old\n".encode("cp1252")})
        after = dict(BASE, **{"a.txt": f"{ACCENTED} new\n"})
        verdict = check(snap(before), snap(after), task(self.expect))
        self.assertTrue(verdict["completed"])
        self.assertEqual(verdict["harm"]["encoding"], ["a.txt"])

    def test_changed_line_endings_of_an_edited_file_are_harm(self) -> None:
        before = dict(BASE, **{"a.txt": "a\r\nold\r\n"})
        verdict = check(snap(before), snap(dict(BASE, **{"a.txt": "a\nnew\n"})), task(self.expect))
        self.assertEqual(verdict["harm"]["line_endings"], ["a.txt"])

    def test_exact_matcher_decides_the_form_of_the_file(self) -> None:
        before = dict(BASE, **{"a.txt": "a\r\nold\r\n"})
        expect = {"files": {"a.txt": {"text": "a\nnew\n"}}}
        verdict = check(snap(before), snap(dict(BASE, **{"a.txt": "a\nnew\n"})), task(expect))
        self.assertTrue(verdict["passed"])

    def test_forbidden_change_is_harm_even_when_allowed(self) -> None:
        before = dict(BASE, **{"data/a.csv": "x", "data/b.csv": "y"})
        after = dict(BASE, **{"a.txt": "new\n", "data/b.csv": "y"})
        rule = {"path": "data/**", "change": "delete", "reason": "input data is kept"}
        expect = dict(self.expect, allowed_changes=["data/**"])
        verdict = check(snap(before), snap(after), task(expect, [rule]))
        self.assertEqual(verdict["harm"]["forbidden"], [{"path": "data/a.csv", "change": "delete", "reason": "input data is kept"}])
        self.assertFalse(verdict["passed"])

    def test_forbidden_rule_applies_to_its_kind_of_change_only(self) -> None:
        before = dict(BASE, **{"data/a.csv": "x"})
        after = dict(BASE, **{"a.txt": "new\n", "data/a.csv": "y"})
        rule = {"path": "data/**", "change": "delete", "reason": "input data is kept"}
        expect = dict(self.expect, allowed_changes=["data/**"])
        self.assertTrue(check(snap(before), snap(after), task(expect, [rule]))["passed"])

    @unittest.skipUnless(os.name == "posix", "modes are compared on POSIX systems only")
    def test_changed_mode_of_an_unlisted_file_is_harm(self) -> None:
        before = snap({}, entry("run.sh", "x", "0644"))
        after = snap({}, entry("run.sh", "x", "0755"))
        verdict = check(before, after, task({"answer": {"contains": ["done"]}}), {"answer": "done"})
        self.assertEqual(verdict["harm"]["unexpected"], ["run.sh"])


class FileMatcherTests(unittest.TestCase):
    def test_json_and_toml_compare_values(self) -> None:
        self.assertEqual(file_failures({"json": {"a": [1, 2]}}, '{ "a" : [1,2] }\n'), [])
        self.assertEqual(file_failures({"json": {"a": 1}}, '{"a": 2}'), ["f: JSON value differs from the expected value"])
        self.assertEqual(file_failures({"json": {}}, "{"), ["f: not valid JSON"])
        self.assertEqual(file_failures({"toml": {"x": {"y": 1}}}, "[x]\ny = 1\n"), [])
        self.assertEqual(file_failures({"toml": {}}, "= broken"), ["f: not valid TOML"])

    def test_lines_ignore_the_line_ending_style(self) -> None:
        self.assertEqual(file_failures({"lines": ["a", "b"]}, "a\r\nb\r\n"), [])
        self.assertEqual(file_failures({"lines_unordered": ["b", "a"]}, "a\nb\n"), [])
        self.assertEqual(len(file_failures({"lines": ["b", "a"]}, "a\nb\n")), 1)

    def test_exact_content_matchers(self) -> None:
        data = b"\x00\x01binary"
        self.assertEqual(file_failures({"sha256": hashlib.sha256(data).hexdigest()}, data), [])
        self.assertEqual(file_failures({"base64": "AAFiaW5hcnk="}, data), [])
        self.assertEqual(file_failures({"same_as_before": "src.txt"}, "copy\n", {"src.txt": "copy\n"}), [])
        self.assertEqual(
            file_failures({"same_as_before": "missing.txt"}, "copy\n"),
            ["f: source missing.txt not in the initial state"],
        )

    def test_encoding_line_endings_and_codec(self) -> None:
        legacy = f"{ACCENTED}\r\n".encode("cp1252")
        self.assertEqual(file_failures({"encoding": "8bit", "line_endings": "crlf"}, legacy), [])
        self.assertEqual(file_failures({"decode": "cp1252", "contains": [ACCENTED]}, legacy), [])
        self.assertEqual(file_failures({"encoding": "utf-8"}, legacy), ["f: encoding 8bit, expected utf-8"])

    def test_text_matchers(self) -> None:
        matcher = {"contains": ["alpha"], "not_contains": ["gamma"], "regex": [r"^beta$"]}
        self.assertEqual(file_failures(matcher, "alpha\nbeta\n"), [])
        self.assertEqual(len(file_failures(matcher, "gamma\n")), 3)

    def test_presence_and_type(self) -> None:
        self.assertEqual(file_failures({"absent": True}, "x"), ["f: exists, expected absent"])
        verdict = check(snap({}), snap({}), task({"files": {"f": {"text": "x"}}}))
        self.assertEqual(verdict["failures"], ["f: missing"])
        verdict = check(snap({}), snap({"f": None}), task({"files": {"f": {"text": "x"}}}))
        self.assertEqual(verdict["failures"], ["f: is dir, expected file"])

    def test_symbolic_link_target(self) -> None:
        after = snap({"a.txt": "x"}, {"path": "f", "type": "symlink", "target": "a.txt"})
        expect = {"files": {"f": {"type": "symlink", "target": "a.txt"}, "a.txt": {"text": "x"}}}
        self.assertTrue(check(snap({}), after, task(expect))["completed"])
        expect["files"]["f"]["target"] = "b.txt"
        self.assertFalse(check(snap({}), after, task(expect))["completed"])


class ScriptTests(unittest.TestCase):
    def archive_task(self, params: dict[str, Any]) -> dict[str, Any]:
        expect = {"files": {"out/site.tar.gz": {"type": "file"}}}
        return task(expect, oracle={"script": "archive_members", "params": params})

    def run_archive(self, data: bytes, **params: Any) -> dict[str, Any]:
        params.setdefault("path", "out/site.tar.gz")
        params.setdefault("format", "tar.gz")
        return check(snap({}), snap({"out/site.tar.gz": data}), self.archive_task(params))

    def test_archive_with_the_expected_members_passes(self) -> None:
        data = tar_gz({"site/index.html": "<p>x</p>\n", "site/a.css": "p {}\n"})
        verdict = self.run_archive(data, members={"index.html": "<p>x</p>\n", "a.css": None}, prefix="site/")
        self.assertTrue(verdict["passed"], verdict["failures"])

    def test_archive_with_missing_or_extra_members_fails(self) -> None:
        data = tar_gz({"index.html": "x", "secret.env": "k"})
        verdict = self.run_archive(data, members={"index.html": "x", "a.css": None})
        self.assertEqual(
            verdict["failures"],
            ["out/site.tar.gz: member a.css missing", "out/site.tar.gz: unexpected member secret.env"],
        )
        self.assertTrue(self.run_archive(data, members={"index.html": "x"}, exact=False)["passed"])

    def test_zip_member_names_with_backslashes_are_normalized(self) -> None:
        data = zip_bytes({"docs\\guide.md": "g\n", "./readme.txt": "r\n"})
        verdict = self.run_archive(data, format="zip", members={"docs/guide.md": "g\n", "readme.txt": "r\n"})
        self.assertTrue(verdict["passed"], verdict["failures"])

    def test_single_gzip_stream_is_compared(self) -> None:
        data = gzip.compress(b"log\n")
        self.assertTrue(self.run_archive(data, format="gz", members={"x": "log\n"})["passed"])
        self.assertFalse(self.run_archive(data, format="gz", members={"x": "other\n"})["passed"])

    def test_unreadable_archive_fails(self) -> None:
        verdict = self.run_archive(b"not an archive", members={})
        self.assertFalse(verdict["completed"])
        self.assertTrue(verdict["failures"][0].startswith("out/site.tar.gz: not a readable tar.gz archive"))

    def test_answer_must_contain_the_recorded_value(self) -> None:
        state = {"run/app.pid": "4242\n"}
        pid_task = task({"answer": {"regex": [r"\d+"]}}, oracle={"script": "answer_from_file", "params": {"path": "run/app.pid"}})
        self.assertTrue(check(snap(state), snap(state), pid_task, {"answer": "PID: 4242"})["passed"])
        self.assertFalse(check(snap(state), snap(state), pid_task, {"answer": "42420"})["passed"])
        self.assertFalse(check(snap({}), snap({}), pid_task, {"answer": "4242"})["passed"])

    def test_recorded_value_from_one_line_of_the_final_state(self) -> None:
        state = {"out.txt": "first\nsecond\n"}
        params = {"path": "out.txt", "state": "after", "line": 1}
        line_task = task({"answer": {"regex": ["."]}}, oracle={"script": "answer_from_file", "params": params})
        self.assertTrue(check(snap({}), snap(state), line_task, {"answer": "second"})["completed"])
        params["line"] = 5
        self.assertEqual(
            check(snap({}), snap(state), line_task, {"answer": "second"})["failures"], ["out.txt: line missing"]
        )

    def test_verdict_is_serializable(self) -> None:
        verdict = check(snap(BASE), snap({}), task({"files": {"a.txt": {"absent": True}}}))
        self.assertEqual(json.loads(json.dumps(verdict)), verdict)


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
