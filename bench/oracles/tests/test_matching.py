# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bench/oracles/tests/test_matching.py
# Responsibility:   Script test_matching.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Tests of path patterns, text form classification and snapshots."""

from __future__ import annotations

import codecs
import datetime
import os
import tempfile
import threading
import unittest
from pathlib import Path

from nurt_bench_oracle.paths import ancestors, matches, matches_any
from nurt_bench_oracle.snapshot import (
    Entry,
    SnapshotError,
    as_index,
    capture,
    check_relative,
    index_from_snapshot,
    materialize,
    normalize_mode,
    resolve_fs_before,
    snapshot_from_index,
)
from nurt_bench_oracle.tasks import FIXTURES_DIR
from nurt_bench_oracle.textform import (
    decode,
    encoding_changed,
    encoding_of,
    line_endings_changed,
    line_endings_of,
    split_lines,
)

from .support import entry, snap

ACCENTED = "caf\u00e9 cr\u00e8me"


class PathPatternTests(unittest.TestCase):
    def test_single_star_stays_within_one_segment(self) -> None:
        self.assertTrue(matches("src/a.py", "src/*.py"))
        self.assertFalse(matches("src/sub/a.py", "src/*.py"))

    def test_double_star_matches_any_number_of_segments(self) -> None:
        self.assertTrue(matches("src/a/b/c.py", "src/**/*.py"))
        self.assertTrue(matches("src/c.py", "src/**/*.py"))
        self.assertTrue(matches("pkg/__pycache__/m.pyc", "**/__pycache__/**"))
        self.assertTrue(matches("__pycache__", "**/__pycache__/**"))

    def test_trailing_double_star_matches_the_directory_itself(self) -> None:
        self.assertTrue(matches("build", "build/**"))
        self.assertTrue(matches("build/x/y", "build/**"))
        self.assertFalse(matches("builder", "build/**"))

    def test_character_classes_and_negation(self) -> None:
        self.assertTrue(matches("log1.txt", "log[0-9].txt"))
        self.assertFalse(matches("logA.txt", "log[0-9].txt"))
        self.assertTrue(matches("logA.txt", "log[!0-9].txt"))
        self.assertTrue(matches("a.c", "a?c"))

    def test_a_bracket_without_a_class_stands_for_itself(self) -> None:
        self.assertTrue(matches("a[].txt", "a[].txt"))
        self.assertTrue(matches("[!]", "[!]"))
        self.assertTrue(matches("a[b", "a[b"))
        self.assertFalse(matches("ab", "a[]"))
        self.assertTrue(matches("a]", "a]"))

    def test_any_of_several_patterns(self) -> None:
        self.assertTrue(matches_any("run/app.pid", ["logs/**", "run/*.pid"]))
        self.assertFalse(matches_any("data/a.csv", ["logs/**", "run/*.pid"]))

    def test_ancestors_are_listed_outermost_first(self) -> None:
        self.assertEqual(ancestors("a/b/c.txt"), ["a", "a/b"])
        self.assertEqual(ancestors("c.txt"), [])


class TextFormTests(unittest.TestCase):
    def test_encoding_is_classified_by_byte_order_mark_and_validity(self) -> None:
        self.assertEqual(encoding_of(b"plain"), "utf-8")
        self.assertEqual(encoding_of(ACCENTED.encode("utf-8")), "utf-8")
        self.assertEqual(encoding_of(codecs.BOM_UTF8 + b"x"), "utf-8-bom")
        self.assertEqual(encoding_of(ACCENTED.encode("utf-16")), "utf-16le")
        self.assertEqual(encoding_of(codecs.BOM_UTF16_BE + "x".encode("utf-16-be")), "utf-16be")
        self.assertEqual(encoding_of(ACCENTED.encode("cp1252")), "8bit")

    def test_ascii_content_is_compatible_with_utf8_and_8bit(self) -> None:
        self.assertFalse(encoding_changed(b"plain", ACCENTED.encode("cp1252")))
        self.assertTrue(encoding_changed(ACCENTED.encode("utf-8"), ACCENTED.encode("cp1252")))
        self.assertTrue(encoding_changed(b"plain", codecs.BOM_UTF8 + b"plain"))
        self.assertFalse(encoding_changed(b"a", b"b"))

    def test_line_ending_styles(self) -> None:
        self.assertEqual(line_endings_of("a\r\nb\r\n"), "crlf")
        self.assertEqual(line_endings_of("a\nb\n"), "lf")
        self.assertEqual(line_endings_of("a\r\nb\n"), "mixed")
        self.assertEqual(line_endings_of("ab"), "none")

    def test_change_of_a_uniform_line_ending_style(self) -> None:
        self.assertTrue(line_endings_changed(b"a\r\nb\r\n", b"a\nb\n"))
        self.assertFalse(line_endings_changed(b"a\r\nb\r\n", b"a\r\nc\r\n"))
        self.assertFalse(line_endings_changed(b"a\r\nb\n", b"a\nb\n"))
        self.assertFalse(line_endings_changed(b"a\r\n", b"abc"))

    def test_lines_do_not_depend_on_the_line_ending_style(self) -> None:
        self.assertEqual(split_lines("a\r\nb\n"), ["a", "b"])
        self.assertEqual(split_lines("a\nb"), ["a", "b"])
        self.assertEqual(split_lines(""), [])

    def test_decoding_follows_the_byte_order_mark_or_the_codec(self) -> None:
        self.assertEqual(decode(codecs.BOM_UTF8 + b"x"), "x")
        self.assertEqual(decode(ACCENTED.encode("utf-16")), ACCENTED)
        self.assertEqual(decode(ACCENTED.encode("cp1252"), "cp1252"), ACCENTED)


class SnapshotTests(unittest.TestCase):
    def test_modes_are_normalized_to_four_octal_digits(self) -> None:
        self.assertEqual(normalize_mode("755"), "0755")
        self.assertEqual(normalize_mode(0o600), "0600")
        self.assertIsNone(normalize_mode(None))
        with self.assertRaises(SnapshotError):
            normalize_mode("rwx")

    def test_rejects_paths_that_leave_the_task_root(self) -> None:
        for path in ("/etc/passwd", "a/../b", "C:/x", "a\\b", "", "a//b", "./a"):
            with self.subTest(path=path), self.assertRaises(SnapshotError):
                check_relative(path)
        self.assertEqual(check_relative("a/b.txt"), "a/b.txt")

    def test_format_member_is_optional(self) -> None:
        raw = {"exhaustive": True, "entries": [entry("a.txt", "x", "0644")]}
        self.assertEqual(index_from_snapshot(raw)["a.txt"].content, b"x")
        self.assertIn("a.txt", index_from_snapshot(dict(raw, format=None)))
        with self.assertRaises(SnapshotError):
            index_from_snapshot(dict(raw, format="other/1"))
        with self.assertRaises(SnapshotError):
            index_from_snapshot({"format": "nurt-fs-snapshot/1"})

    def test_rejects_malformed_entries(self) -> None:
        both = {"path": "a", "type": "file", "content_utf8": "x", "content_base64": "eA=="}
        for raw in (both, {"path": "l", "type": "symlink"}, {"path": "x", "type": "socket"}):
            with self.subTest(raw=raw), self.assertRaises(SnapshotError):
                index_from_snapshot({"entries": [raw]})

    def test_implicit_parent_directories_are_added(self) -> None:
        index = as_index(snap({"a/b/c.txt": "x"}))
        self.assertEqual(index["a"].kind, "dir")
        self.assertEqual(index["a/b"].kind, "dir")

    def test_snapshot_round_trip_keeps_binary_content(self) -> None:
        data = snap({"bin": b"\xff\x00", "text.txt": "x\n", "dir": None})
        again = index_from_snapshot(snapshot_from_index(index_from_snapshot(data)))
        self.assertEqual(again["bin"].content, b"\xff\x00")
        self.assertEqual(again["text.txt"].content, b"x\n")
        self.assertEqual(again["dir"].kind, "dir")

    def test_fixture_reference_with_overlay(self) -> None:
        overlay = [entry("notes.txt", "a\n")]
        index = resolve_fs_before({"fixture": "py-inventory", "overlay": overlay}, FIXTURES_DIR)
        self.assertEqual(index["notes.txt"].content, b"a\n")
        self.assertIn("pyproject.toml", index)
        with self.assertRaises(SnapshotError):
            resolve_fs_before({"fixture": "no-such-fixture"}, FIXTURES_DIR)

    def test_materialize_and_capture_round_trip(self) -> None:
        index = as_index(snap({"a/b.txt": "x\n", "bin": b"\x00\xff", "empty": None}))
        index["a/b.txt"] = Entry("file", b"x\n", "0640", mtime="2026-01-02T03:04:05Z")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            materialize(index, root)
            captured = capture(root)
            stamp = datetime.datetime(2026, 1, 2, 3, 4, 5, tzinfo=datetime.timezone.utc).timestamp()
            self.assertEqual(os.stat(root / "a" / "b.txt").st_mtime, stamp)
        self.assertEqual(set(captured), {"a", "a/b.txt", "bin", "empty"})
        self.assertEqual(captured["bin"].content, b"\x00\xff")
        if os.name == "posix":
            self.assertEqual(captured["a/b.txt"].mode, "0640")
            self.assertEqual(captured["bin"].mode, "0644")

    @unittest.skipUnless(os.name == "posix", "symbolic links need POSIX")
    def test_symbolic_links_are_captured_without_following(self) -> None:
        raw = snap({"target.txt": "x"}, {"path": "link", "type": "symlink", "target": "target.txt"})
        with tempfile.TemporaryDirectory() as tmp:
            materialize(as_index(raw), Path(tmp))
            captured = capture(Path(tmp))
        self.assertEqual(captured["link"].kind, "symlink")
        self.assertEqual(captured["link"].target, "target.txt")

    @unittest.skipUnless(os.name == "posix", "symbolic links need POSIX")
    def test_an_entry_below_a_symbolic_link_is_not_written_through_it(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            outside = Path(tmp) / "outside"
            outside.mkdir()
            root = Path(tmp) / "root"
            root.mkdir()
            raw = snap({}, {"path": "link", "type": "symlink", "target": str(outside)}, entry("link/x.txt", "x"))
            with self.assertRaises(SnapshotError):
                materialize(as_index(raw), root)
            self.assertEqual(list(outside.iterdir()), [])

    @unittest.skipUnless(hasattr(os, "mkfifo"), "named pipes need POSIX")
    def test_capture_records_a_pipe_without_opening_it(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "plain.txt").write_text("x", encoding="utf-8")
            os.mkfifo(root / "pipe")
            result: dict[str, dict[str, Entry]] = {}
            worker = threading.Thread(target=lambda: result.update(index=capture(root)), daemon=True)
            worker.start()
            worker.join(10)
            if worker.is_alive():
                # Release a reader that is blocked in open() so that the directory can be removed.
                os.close(os.open(root / "pipe", os.O_RDWR))
                worker.join(10)
                self.fail("capture blocked on a named pipe")
        self.assertEqual(result["index"]["pipe"].kind, "special")
        self.assertEqual(result["index"]["plain.txt"].content, b"x")


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
