# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Tests of packaging/install/install.sh against a local release directory."""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import support

SCRIPT = support.PACKAGING / "install" / "install.sh"


class InstallScriptTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.releases = self.root / "releases"
        self.target = support.host_target()
        self.archives = support.make_release_tree(self.releases)
        self.bin_dir = self.root / "bin"

    def install(self, *arguments, env=None, releases=None):
        environment = support.clean_environment(
            NURT_RELEASES_URL=str(releases or self.releases), HOME=str(self.root / "home")
        )
        environment.update(env or {})
        return subprocess.run(
            ["sh", str(SCRIPT), "--bin-dir", str(self.bin_dir), *arguments],
            capture_output=True,
            text=True,
            env=environment,
            check=False,
        )

    def installed_version_line(self):
        result = subprocess.run(
            [str(self.bin_dir / "nurt"), "version"], capture_output=True, text=True, check=False
        )
        self.assertEqual(result.returncode, 0)
        return result.stdout.splitlines()[0]

    def test_installs_latest_release_and_runs(self):
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.installed_version_line().startswith("nurt 0.1.0"))
        self.assertIn(f"installed nurt {support.VERSION}", result.stderr)

    def test_falls_back_to_the_glibc_archive_when_the_release_has_no_musl_one(self):
        if self.target.endswith("apple-darwin"):
            self.skipTest("macOS has one target")
        releases = self.root / "glibc-only"
        glibc = self.target.replace("-musl", "-gnu")
        support.make_release_tree(releases, targets=(glibc, "x86_64-pc-windows-msvc"))
        result = self.install(releases=releases)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.installed_version_line().startswith("nurt 0.1.0"))

    def test_installs_requested_version(self):
        result = self.install("--version", f"v{support.VERSION}")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.bin_dir / "nurt").is_file())

    def test_installs_into_default_directory_under_home(self):
        environment = support.clean_environment(
            NURT_RELEASES_URL=str(self.releases), HOME=str(self.root / "home")
        )
        result = subprocess.run(
            ["sh", str(SCRIPT)], capture_output=True, text=True, env=environment, check=False
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.root / "home" / ".local" / "bin" / "nurt").is_file())

    def test_reads_location_from_file_url(self):
        result = self.install(releases=f"file://{self.releases}")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_rejects_archive_with_wrong_digest(self):
        archive = self.releases / "download" / f"v{support.VERSION}" / f"nurt-{support.VERSION}-{self.target}.tar.gz"
        archive.write_bytes(b"tampered")
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("SHA-256 mismatch", result.stderr)
        self.assertFalse((self.bin_dir / "nurt").exists())

    def test_rejects_unknown_version(self):
        result = self.install("--version", "9.9.9")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.bin_dir / "nurt").exists())

    def test_rejects_malformed_version(self):
        result = self.install("--version", "1.0.0;rm")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("invalid version", result.stderr)

    def test_rejects_malformed_version_named_by_the_latest_release(self):
        latest = self.releases / "latest" / "download" / "checksums.txt"
        latest.write_text(f"{'0' * 64}  nurt-../evil-{self.target}.tar.gz\n", encoding="utf-8")
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("invalid version in the latest release", result.stderr)
        self.assertFalse((self.bin_dir / "nurt").exists())

    def test_rejects_unsupported_architecture(self):
        shim = self.root / "shim"
        shim.mkdir()
        uname = shim / "uname"
        uname.write_text('#!/bin/sh\n[ "$1" = -m ] && echo riscv64 || echo Linux\n', encoding="utf-8")
        uname.chmod(0o755)
        result = self.install(env={"PATH": f"{shim}:{os.environ['PATH']}"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unsupported architecture: riscv64", result.stderr)

    def test_rejects_unsupported_operating_system(self):
        shim = self.root / "shim"
        shim.mkdir()
        uname = shim / "uname"
        uname.write_text('#!/bin/sh\n[ "$1" = -m ] && echo x86_64 || echo FreeBSD\n', encoding="utf-8")
        uname.chmod(0o755)
        result = self.install(env={"PATH": f"{shim}:{os.environ['PATH']}"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unsupported operating system: FreeBSD", result.stderr)

    def test_replaces_existing_executable(self):
        self.bin_dir.mkdir()
        (self.bin_dir / "nurt").write_text("old", encoding="utf-8")
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.installed_version_line().startswith("nurt 0.1.0"))

    def test_requires_valid_signature_when_public_key_is_given(self):
        if shutil.which("gpg") is None:
            self.skipTest("gpg is not installed")
        key = self.root / "key.asc"
        key.write_text("not a key\n", encoding="utf-8")
        signature = self.releases / "download" / f"v{support.VERSION}" / "checksums.txt.asc"
        signature.write_text("not a signature\n", encoding="utf-8")
        result = self.install("--public-key", str(key))
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.bin_dir / "nurt").exists())

    def test_reports_skipped_signature_check_without_public_key(self):
        result = self.install()
        self.assertIn("signature not verified", result.stderr)


class ShellcheckTest(unittest.TestCase):
    def test_script_passes_shellcheck(self):
        program = shutil.which("shellcheck")
        if program is None:
            self.skipTest("shellcheck is not installed")
        for script in (SCRIPT, support.PACKAGING / "deb" / "build-deb.sh", support.PACKAGING / "tests" / "container-install.sh"):
            result = subprocess.run([program, "-s", "sh", str(script)], capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stdout)


if __name__ == "__main__":
    unittest.main()
