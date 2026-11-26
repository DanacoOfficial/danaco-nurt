# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Tests of tools/ci/check-dependencies.sh."""

import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

import support

SCRIPT = support.PACKAGING.parent / "tools" / "ci" / "check-dependencies.sh"

DYNAMIC = """
Dynamic section at offset 0x2d8 contains 9 entries:
  Tag        Type                         Name/Value
 0x0000000000000001 (NEEDED)             Shared library: [{names[0]}]
{rest}
 0x000000000000000c (INIT)               0x1000
"""


def dynamic_section(names):
    rest = "\n".join(f" 0x0000000000000001 (NEEDED)             Shared library: [{name}]" for name in names[1:])
    return DYNAMIC.format(names=names, rest=rest) if names else "\nThere is no dynamic section in this file.\n"


@unittest.skipIf(shutil.which("bash") is None, "bash is not installed")
class CheckDependenciesTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def elf(self, name, needed):
        """Create an ELF-looking file whose fake readelf output names the libraries."""
        path = self.root / name
        path.write_bytes(b"\x7fELF" + b"\0" * 60)
        (self.root / f"{name}.dynamic").write_text(dynamic_section(needed), encoding="utf-8")
        return path

    def fake_readelf(self):
        program = self.root / "readelf"
        program.write_text(
            '#!/bin/sh\n[ "$1" = -d ] || exit 2\nshift\n[ "$1" = -- ] && shift\ncat "$1.dynamic"\n',
            encoding="utf-8",
        )
        program.chmod(program.stat().st_mode | stat.S_IXUSR)
        return program

    def run_script(self, *arguments, readelf=True):
        environment = support.clean_environment()
        if readelf:
            environment["READELF"] = str(self.fake_readelf())
        return subprocess.run(
            ["bash", str(SCRIPT), *arguments], capture_output=True, text=True, env=environment, check=False
        )

    def test_accepts_a_binary_that_needs_only_the_permitted_libraries(self):
        binary = self.elf("nurt", ["libc.so.6", "libm.so.6", "libgcc_s.so.1", "ld-linux-x86-64.so.2"])
        result = self.run_script(str(binary))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"ok: {binary}", result.stdout)

    def test_accepts_a_statically_linked_binary(self):
        result = self.run_script(str(self.elf("nurt", [])))
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_rejects_liblzma(self):
        binary = self.elf("nurt", ["libc.so.6", "liblzma.so.5"])
        result = self.run_script(str(binary))
        self.assertEqual(result.returncode, 1)
        self.assertIn("liblzma.so.5", result.stderr)
        self.assertNotIn("ok:", result.stdout)

    def test_reports_every_file_and_fails_when_one_is_rejected(self):
        good = self.elf("good", ["libc.so.6"])
        bad = self.elf("bad", ["libz.so.1"])
        result = self.run_script(str(good), str(bad))
        self.assertEqual(result.returncode, 1)
        self.assertIn(f"ok: {good}", result.stdout)
        self.assertIn("libz.so.1", result.stderr)

    def test_rejects_a_missing_file(self):
        result = self.run_script(str(self.root / "missing"))
        self.assertEqual(result.returncode, 1)
        self.assertIn("is not a file", result.stderr)

    def test_rejects_a_file_that_is_neither_elf_nor_pe(self):
        path = self.root / "text"
        path.write_text("text\n", encoding="utf-8")
        result = self.run_script(str(path))
        self.assertEqual(result.returncode, 1)
        self.assertIn("neither an ELF nor a PE file", result.stderr)

    def test_rejects_invalid_arguments(self):
        self.assertEqual(self.run_script().returncode, 2)
        self.assertEqual(self.run_script("--format", "macho", "x").returncode, 2)
        self.assertEqual(self.run_script("--unknown", "x").returncode, 2)

    def test_fails_when_readelf_fails(self):
        binary = self.elf("nurt", ["libc.so.6"])
        failing = self.root / "failing"
        failing.write_text("#!/bin/sh\necho broken >&2\nexit 1\n", encoding="utf-8")
        failing.chmod(0o755)
        environment = support.clean_environment(READELF=str(failing))
        result = subprocess.run(
            ["bash", str(SCRIPT), str(binary)], capture_output=True, text=True, env=environment, check=False
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("failed", result.stderr)

    @unittest.skipIf(shutil.which("readelf") is None, "readelf is not installed")
    def test_reads_a_real_executable(self):
        shell = Path(shutil.which("sh")).resolve()
        result = self.run_script(str(shell), readelf=False)
        self.assertIn(result.returncode, (0, 1), result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        if result.returncode == 1:
            self.assertIn("not in the list of permitted libraries", result.stderr)


if __name__ == "__main__":
    unittest.main()
