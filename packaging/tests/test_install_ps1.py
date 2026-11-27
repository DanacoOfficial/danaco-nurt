# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           packaging/tests/test_install_ps1.py
# Responsibility:   Script test_install_ps1.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Checks of the text of install.ps1 for rules that cannot run on every host.

PowerShell with the Windows registry is not available to the test runner, so
the script is checked for the calls that keep the PATH of the user intact.
"""

import shutil
import subprocess
import unittest

import support

SCRIPT = support.PACKAGING / "install" / "install.ps1"


class InstallPs1Test(unittest.TestCase):
    def setUp(self):
        self.text = SCRIPT.read_text(encoding="utf-8")

    def test_path_is_read_and_written_without_expanding_variables(self):
        self.assertIn("DoNotExpandEnvironmentNames", self.text)
        self.assertIn("RegistryValueKind]::ExpandString", self.text)
        self.assertNotIn("SetEnvironmentVariable('Path'", self.text)

    def test_machine_scope_is_limited_to_program_files(self):
        self.assertIn("installs only below Program Files", self.text)

    def test_file_urls_with_a_drive_letter_lose_the_leading_slash(self):
        self.assertIn("'^file:///(?=[A-Za-z]:)'", self.text)

    @unittest.skipIf(shutil.which("pwsh") is None, "pwsh is not installed")
    def test_script_parses(self):
        command = (
            "$errors = $null; "
            f"[System.Management.Automation.Language.Parser]::ParseFile('{SCRIPT}', [ref]$null, [ref]$errors) | Out-Null; "
            "if ($errors) { $errors | ForEach-Object { $_.Message }; exit 1 }"
        )
        result = subprocess.run(
            ["pwsh", "-NoProfile", "-Command", command],
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
