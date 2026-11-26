# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Tests of the container files of the benchmark: pinned bases and verified downloads."""

from __future__ import annotations

import re
import unittest
from pathlib import Path

CONTAINERS = Path(__file__).resolve().parents[2] / "protocol" / "containers"
DIGEST = r"sha256:[0-9a-f]{64}"


class ContainerFilesTest(unittest.TestCase):
    def read(self, name: str) -> str:
        return (CONTAINERS / name).read_text(encoding="utf-8")

    def test_base_images_are_named_by_digest(self):
        for name in ("linux/Containerfile", "windows/Containerfile"):
            froms = re.findall(r"(?m)^FROM\s+(\S+)", self.read(name))
            self.assertEqual(len(froms), 1, name)
            self.assertRegex(froms[0], rf"@{DIGEST}$", name)

    def test_windows_downloads_are_checked_against_a_digest(self):
        text = self.read("windows/Containerfile")
        downloads = re.findall(r"-OutFile (\S+?);", text)
        self.assertEqual(sorted(downloads), ["git.zip", "pwsh.msi", "python.exe"])
        for file in downloads:
            self.assertRegex(text, rf"Get-FileHash {re.escape(file)} -Algorithm SHA256", file)
        for argument in re.findall(r"(?m)^ARG (\w+_SHA256)=(\S+)$", text):
            self.assertRegex(argument[1], r"^[0-9a-f]{64}$", argument[0])
        self.assertEqual(len(re.findall(r"(?m)^ARG \w+_SHA256=", text)), 3)


if __name__ == "__main__":
    unittest.main()
