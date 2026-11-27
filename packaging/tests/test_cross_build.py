# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           packaging/tests/test_cross_build.py
# Responsibility:   Script test_cross_build.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Tests of the cross-built release targets: the zig wrappers and the target list of build-release.sh."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

import support

RELEASE = support.PACKAGING / "release"
BUILD_RELEASE = support.PACKAGING.parent / "tools" / "release" / "build-release.sh"

AARCH64_WRAPPERS = {
    "zig-cc-aarch64-linux-gnu.sh": "aarch64-linux-gnu.2.31",
    "zig-cc-aarch64-linux-musl.sh": "aarch64-linux-musl",
}


def run_wrapper(wrapper, *arguments):
    with tempfile.TemporaryDirectory() as directory:
        zig = Path(directory) / "zig"
        zig.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
        zig.chmod(0o755)
        return subprocess.run(
            ["sh", str(RELEASE / wrapper), *arguments],
            capture_output=True,
            text=True,
            check=False,
            env={"ZIG": str(zig), "PATH": os.environ["PATH"]},
        )


class ZigWrapperTest(unittest.TestCase):
    def test_the_wrappers_are_valid_executable_shell_with_a_rights_header(self):
        for name in (*AARCH64_WRAPPERS, "zig-cc-musl.sh", "zig-ar.sh"):
            with self.subTest(script=name):
                path = RELEASE / name
                done = subprocess.run(["sh", "-n", str(path)], capture_output=True, text=True, check=False)
                self.assertEqual(done.returncode, 0, done.stderr)
                self.assertTrue(os.access(path, os.X_OK))
                head = path.read_text(encoding="utf-8").splitlines()[:4]
                self.assertEqual(head[0], "#!/bin/sh")
                self.assertIn("Copyright (c) 2026 Danaco Official", head[1])

    def test_the_aarch64_wrappers_select_the_target_and_drop_the_options_zig_rejects(self):
        for name, target in AARCH64_WRAPPERS.items():
            with self.subTest(script=name):
                done = run_wrapper(
                    name,
                    "--target=aarch64-unknown-linux-gnu",
                    "-Wl,--fix-cortex-a53-843419",
                    "-Wl,-O1",
                    "-Wl,--as-needed",
                    "-o",
                    "out",
                )
                self.assertEqual(done.returncode, 0, done.stderr)
                self.assertEqual(done.stdout.split(), ["cc", "-target", target, "-Wl,--as-needed", "-o", "out"])

    def test_the_archiver_wrapper_runs_zig_ar(self):
        done = run_wrapper("zig-ar.sh", "rcs", "libx.a", "x.o")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(done.stdout.split(), ["ar", "rcs", "libx.a", "x.o"])


class BuildReleaseTargetsTest(unittest.TestCase):
    def setUp(self):
        self.script = BUILD_RELEASE.read_text(encoding="utf-8")

    def test_every_target_of_the_release_is_listed(self):
        for target in (
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl",
            "aarch64-unknown-linux-gnu",
            "aarch64-unknown-linux-musl",
            "x86_64-pc-windows-msvc",
            "aarch64-pc-windows-msvc",
        ):
            self.assertIn(target, self.script)

    def test_the_script_uses_the_wrappers_and_sets_the_cross_environment(self):
        for name in (*AARCH64_WRAPPERS, "zig-cc-musl.sh", "zig-ar.sh"):
            self.assertIn(name, self.script)
        for text in ('"CC_$name"', '"AR_$name"', '"CFLAGS_$name"', "-fno-sanitize=undefined", "CARGO_TARGET_${upper}_LINKER"):
            self.assertIn(text, self.script)

    def test_the_executable_is_run_for_the_host_target_only(self):
        self.assertIn('if [ "$target" = "$host" ]; then', self.script)
        self.assertNotIn("tar -xzf \"$output/nurt-$version-$LINUX_GNU.tar.gz\"", self.script)

    def test_the_readme_lists_the_wrappers(self):
        readme = (BUILD_RELEASE.parent / "README.md").read_text(encoding="utf-8")
        for name in (*AARCH64_WRAPPERS, "zig-cc-musl.sh", "zig-ar.sh"):
            self.assertIn(name, readme)
        self.assertIn("aarch64-pc-windows-msvc", readme)


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
