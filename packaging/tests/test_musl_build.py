# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           packaging/tests/test_musl_build.py
# Responsibility:   Script test_musl_build.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Tests of the build of the musl target with zig cc: the compiler wrapper and the release workflow."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

import support

ROOT = support.PACKAGING.parent
WRAPPER = support.PACKAGING / "release" / "zig-cc-musl.sh"


class MuslCompilerWrapperTest(unittest.TestCase):
    def run_wrapper(self, *arguments):
        with tempfile.TemporaryDirectory() as directory:
            zig = Path(directory) / "zig"
            zig.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
            zig.chmod(0o755)
            return subprocess.run(
                ["sh", str(WRAPPER), *arguments],
                capture_output=True,
                text=True,
                check=False,
                env={"ZIG": str(zig), "PATH": os.environ["PATH"]},
            )

    def test_the_wrapper_is_valid_shell_and_executable(self):
        done = subprocess.run(["sh", "-n", str(WRAPPER)], capture_output=True, text=True, check=False)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(os.access(WRAPPER, os.X_OK))

    def test_the_wrapper_drops_the_target_option_and_selects_the_musl_target(self):
        done = self.run_wrapper("--target=x86_64-unknown-linux-musl", "-c", "a.c")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(done.stdout.split(), ["cc", "-target", "x86_64-linux-musl", "-c", "a.c"])


class MuslWorkflowTest(unittest.TestCase):
    def test_the_release_workflow_builds_the_musl_target_with_zig_cc(self):
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        mirror = (support.PACKAGING / "ci" / "release.yml").read_text(encoding="utf-8")
        self.assertEqual(workflow, mirror)
        for text in (
            'rustup target add "$MUSL_TARGET"',
            "CC_x86_64_unknown_linux_musl: ${{ github.workspace }}/packaging/release/zig-cc-musl.sh",
            "CFLAGS_x86_64_unknown_linux_musl: -fno-sanitize=undefined",
        ):
            self.assertIn(text, workflow)
        self.assertNotIn("CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER", workflow)

    def test_the_readme_describes_the_wrapper(self):
        readme = (support.PACKAGING / "README.md").read_text(encoding="utf-8")
        self.assertIn("packaging/release/zig-cc-musl.sh", readme)


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
