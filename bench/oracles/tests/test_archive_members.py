# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Tests of the oracle script that checks the members of an archive."""

from __future__ import annotations

import gzip
import io
import lzma
import tarfile
import unittest
import zipfile
from typing import Any
from unittest import mock

from nurt_bench_oracle.scripts import archive_members
from nurt_bench_oracle.snapshot import Entry


def tar_bytes(files: dict[str, bytes]) -> bytes:
    """Returns an uncompressed tar archive of regular files."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as archive:
        for name, content in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(content)
            archive.addfile(info, io.BytesIO(content))
    return buffer.getvalue()


def zip_bytes(files: dict[str, bytes]) -> bytes:
    """Returns a zip archive of the files."""
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, content in files.items():
            archive.writestr(name, content)
    return buffer.getvalue()


def verdict(data: bytes | None, **params: Any) -> dict[str, Any]:
    """Runs the script on an archive stored as ``out`` in the final state."""
    after = {} if data is None else {"out": Entry("file", data)}
    task = {"oracle": {"script": "archive_members", "params": {"path": "out", **params}}}
    return archive_members.check({}, after, task, {})


class ArchiveMemberTests(unittest.TestCase):
    def test_a_zip_with_the_expected_members_completes(self) -> None:
        data = zip_bytes({"a.txt": b"alpha\n", "dir/b.txt": b"beta\n"})
        result = verdict(data, format="zip", members={"a.txt": "alpha\n", "dir/b.txt": None})
        self.assertEqual(result, {"completed": True, "failures": []})

    def test_a_missing_or_different_member_and_an_extra_member_fail(self) -> None:
        data = tar_bytes({"a.txt": b"alpha\n", "extra.txt": b"x"})
        result = verdict(data, format="tar", members={"a.txt": "other\n", "b.txt": None})
        self.assertFalse(result["completed"])
        self.assertEqual(
            sorted(result["failures"]),
            [
                "out: member a.txt has different content",
                "out: member b.txt missing",
                "out: unexpected member extra.txt",
            ],
        )

    def test_exact_false_allows_other_members_and_a_prefix_is_removed(self) -> None:
        data = tar_bytes({"./pkg/a.txt": b"alpha\n", "pkg/extra.txt": b"x"})
        result = verdict(data, format="tar", members={"a.txt": "alpha\n"}, exact=False, prefix="pkg/")
        self.assertEqual(result, {"completed": True, "failures": []})

    def test_compressed_tar_archives_are_read(self) -> None:
        tar = tar_bytes({"a.txt": b"alpha\n"})
        for kind, data in (("tar.gz", gzip.compress(tar)), ("tar.xz", lzma.compress(tar))):
            with self.subTest(kind=kind):
                result = verdict(data, format=kind, members={"a.txt": "alpha\n"})
                self.assertTrue(result["completed"], result)

    def test_a_gz_stream_with_several_members_is_joined(self) -> None:
        data = gzip.compress(b"ab") + gzip.compress(b"cd")
        self.assertTrue(verdict(data, format="gz", members={"x": "abcd"})["completed"])
        self.assertFalse(verdict(data, format="gz", members={"x": "ab"})["completed"])

    def test_a_missing_archive_is_reported(self) -> None:
        result = verdict(None, format="zip", members={})
        self.assertEqual(result, {"completed": False, "failures": ["out: archive missing"]})

    def test_unreadable_archives_fail_instead_of_raising(self) -> None:
        packed = gzip.compress(b"hello world, hello world")
        cases = {
            "corrupt deflate stream": (packed[:10] + b"\xff\xff\xff\xff", "gz"),
            "truncated gzip": (packed[:-6], "gz"),
            "not gzip": (b"plain text", "gz"),
            "corrupt xz": (lzma.compress(b"abc")[:20], "tar.xz"),
            "not a zip": (b"PK nothing", "zip"),
            "not a tar": (gzip.compress(b"x" * 600), "tar.gz"),
        }
        for label, (data, kind) in cases.items():
            with self.subTest(label):
                result = verdict(data, format=kind, members={"x": None})
                self.assertFalse(result["completed"])
                self.assertIn("not a readable", result["failures"][0])

    def test_content_beyond_the_limit_is_not_unpacked(self) -> None:
        with mock.patch.object(archive_members, "MAX_UNPACKED_BYTES", 10):
            for kind, data in (
                ("gz", gzip.compress(b"\0" * 100)),
                ("tar.xz", lzma.compress(tar_bytes({"a": b"\0" * 100}))),
                ("zip", zip_bytes({"a": b"\0" * 100})),
            ):
                with self.subTest(kind):
                    result = verdict(data, format=kind, members={"a": None})
                    self.assertFalse(result["completed"])
                    self.assertIn("permitted amount", result["failures"][0])

    def test_names_are_normalised(self) -> None:
        self.assertEqual(archive_members.normalize_name("./a\\b.txt", ""), "a/b.txt")
        self.assertEqual(archive_members.normalize_name("/pkg/a", "/pkg/"), "a")
        self.assertEqual(archive_members.normalize_name("pkg/a", "pkg/"), "a")


if __name__ == "__main__":
    unittest.main()
