# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Tests of tools/ci/check-dependencies.ps1 with a stand-in for objdump."""

import os
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

import support

SCRIPT = support.PACKAGING.parent / "tools" / "ci" / "check-dependencies.ps1"
POWERSHELL = shutil.which("pwsh")

SYSTEM_ONLY = ["KERNEL32.dll", "ntdll.dll", "bcrypt.dll", "api-ms-win-core-synch-l1-2-0.dll", "WS2_32.dll"]


@unittest.skipIf(POWERSHELL is None, "pwsh is not installed")
@unittest.skipIf(shutil.which("dumpbin") is not None, "dumpbin on the path replaces the stand-in")
class CheckDependenciesPs1Test(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.file = self.root / "nurt.exe"
        self.file.write_bytes(b"MZ")

    def run_script(self, imports, *files):
        listing = "".join(f"\tDLL Name: {name}\n" for name in imports)
        for name in ("objdump", "llvm-objdump"):
            fake = self.bin / name
            fake.write_text(f"#!/bin/sh\nprintf '%s' '{listing}'\n", encoding="utf-8")
            fake.chmod(fake.stat().st_mode | stat.S_IXUSR)
        environment = dict(os.environ, PATH=f"{self.bin}{os.pathsep}{os.environ['PATH']}")
        return subprocess.run(
            [POWERSHELL, "-NoProfile", "-File", str(SCRIPT), *map(str, files or (self.file,))],
            capture_output=True, text=True, env=environment, check=False,
        )

    def test_system_dlls_pass(self):
        result = self.run_script(SYSTEM_ONLY)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("ok (pe, 5 dependencies)", result.stdout)

    def test_the_visual_cpp_runtime_is_refused(self):
        result = self.run_script(SYSTEM_ONLY + ["VCRUNTIME140.dll"])
        self.assertEqual(result.returncode, 1)
        self.assertIn("vcruntime140.dll", result.stderr)

    def test_the_universal_crt_forwarders_are_refused(self):
        result = self.run_script(SYSTEM_ONLY + ["api-ms-win-crt-stdio-l1-1-0.dll", "ucrtbase.dll"])
        self.assertEqual(result.returncode, 1)
        self.assertIn("api-ms-win-crt-stdio-l1-1-0.dll", result.stderr)
        self.assertIn("ucrtbase.dll", result.stderr)

    def test_an_unknown_library_is_refused(self):
        result = self.run_script(SYSTEM_ONLY + ["libfoo.dll"])
        self.assertEqual(result.returncode, 1)
        self.assertIn("libfoo.dll", result.stderr)

    def test_a_missing_file_and_an_empty_import_table_fail(self):
        self.assertEqual(self.run_script(SYSTEM_ONLY, self.root / "absent.exe").returncode, 1)
        self.assertEqual(self.run_script([]).returncode, 1)


if __name__ == "__main__":
    unittest.main()
