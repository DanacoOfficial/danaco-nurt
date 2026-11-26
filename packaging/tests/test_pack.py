# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Tests of packaging/release/pack.py."""

import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

import support

SCRIPT = support.PACKAGING / "release" / "pack.py"


class PackTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repository = self.root / "repository"
        (self.repository / "crates" / "nurt-ffi" / "include").mkdir(parents=True)
        (self.repository / "crates" / "nurt-ffi" / "include" / "nurt.h").write_text("/* header */\n")
        for name in ("LICENSE", "NOTICE", "README.md", "CHANGELOG.md"):
            (self.repository / name).write_text(f"{name}\n")
        self.binary = self.root / "nurt"
        self.binary.write_bytes(b"executable")
        self.library = self.root / "libnurt_ffi.so"
        self.library.write_bytes(b"library")
        self.man = self.root / "man"
        self.man.mkdir()
        (self.man / "nurt.1").write_text(".TH NURT 1\n")
        (self.man / "nurt-journal.1").write_text(".TH NURT-JOURNAL 1\n")
        self.completions = self.root / "completions"
        self.completions.mkdir()
        (self.completions / "nurt.bash").write_text("complete -F _nurt nurt\n")
        (self.completions / "_nurt.ps1").write_text("Register-ArgumentCompleter\n")
        self.site = self.root / "site"
        (self.site / "commands").mkdir(parents=True)
        (self.site / "index.html").write_text("<html></html>\n")
        (self.site / "commands" / "find.html").write_text("<html>find</html>\n")

    def pack(self, target, output, *extra):
        return subprocess.run(
            [
                sys.executable, "-I", str(SCRIPT),
                "--root", str(self.repository), "--version", "0.1.0", "--target", target,
                "--binary", str(self.binary), "--output", str(output),
                "--completions", str(self.completions), "--site", str(self.site),
                "--epoch", "1700000000", *extra,
            ],
            capture_output=True, text=True, check=False,
        )

    def names(self, path):
        if path.suffix == ".zip":
            with zipfile.ZipFile(path) as archive:
                return set(archive.namelist())
        with tarfile.open(path) as archive:
            return {member.name for member in archive.getmembers() if member.isfile()}

    def test_linux_program_archive_has_the_documented_layout(self):
        target = "x86_64-unknown-linux-gnu"
        result = self.pack(target, self.root / "out", "--library", str(self.library), "--man", str(self.man))
        self.assertEqual(result.returncode, 0, result.stderr)
        top = f"nurt-0.1.0-{target}"
        with tarfile.open(self.root / "out" / f"{top}.tar.gz") as archive:
            names = {member.name for member in archive.getmembers() if member.isfile()}
            self.assertEqual(archive.getmember(f"{top}/nurt").mode, 0o755)
            self.assertEqual(archive.getmember(f"{top}/nurt").uid, 0)
        expected = {
            "nurt", "LICENSE", "NOTICE", "README.md",
            "CHANGELOG.md", "man/man1/nurt.1", "man/man1/nurt-journal.1",
            "completions/nurt.bash", "completions/_nurt.ps1", "doc/index.html",
            "doc/commands/find.html",
        }
        self.assertEqual(names, {f"{top}/{name}" for name in expected})

    def test_linux_library_archive_holds_the_library_header_and_pkg_config(self):
        target = "x86_64-unknown-linux-gnu"
        result = self.pack(target, self.root / "out", "--library", str(self.library), "--man", str(self.man))
        self.assertEqual(result.returncode, 0, result.stderr)
        top = f"nurt-lib-0.1.0-{target}"
        with tarfile.open(self.root / "out" / f"{top}.tar.gz") as archive:
            names = {member.name for member in archive.getmembers() if member.isfile()}
            pc = archive.extractfile(f"{top}/nurt.pc").read().decode()
        expected = {"libnurt.so.1", "nurt.h", "nurt.pc", "LICENSE", "NOTICE"}
        self.assertEqual(names, {f"{top}/{name}" for name in expected})
        for variable in ("prefix", "libdir", "includedir"):
            self.assertIn(f"\n{variable}=" if variable != "prefix" else "prefix=", pc)
        self.assertIn("Version: 0.1.0", pc)

    def test_no_library_archive_without_a_library(self):
        target = "x86_64-unknown-linux-gnu"
        self.assertEqual(self.pack(target, self.root / "out").returncode, 0)
        self.assertEqual(sorted(path.name for path in (self.root / "out").iterdir()), [f"nurt-0.1.0-{target}.tar.gz"])

    def test_windows_program_archive_has_the_files_at_the_top_level(self):
        target = "x86_64-pc-windows-msvc"
        result = self.pack(target, self.root / "out", "--library", str(self.library), "--man", str(self.man))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            self.names(self.root / "out" / f"nurt-0.1.0-{target}.zip"),
            {"nurt.exe", "LICENSE", "NOTICE", "README.md", "CHANGELOG.md", "completions/nurt.bash",
             "completions/_nurt.ps1", "doc/index.html", "doc/commands/find.html"},
        )
        self.assertEqual(
            self.names(self.root / "out" / f"nurt-lib-0.1.0-{target}.zip"),
            {"nurt.dll", "nurt.h", "LICENSE", "NOTICE"},
        )

    def test_archives_are_reproducible(self):
        for target, suffix in (("x86_64-unknown-linux-gnu", "tar.gz"), ("x86_64-pc-windows-msvc", "zip")):
            first, second = self.root / "first", self.root / "second"
            self.assertEqual(self.pack(target, first).returncode, 0)
            self.assertEqual(self.pack(target, second).returncode, 0)
            name = f"nurt-0.1.0-{target}.{suffix}"
            self.assertEqual((first / name).read_bytes(), (second / name).read_bytes())

    def test_rejects_a_missing_input(self):
        (self.repository / "NOTICE").unlink()
        result = self.pack("x86_64-unknown-linux-gnu", self.root / "out")
        self.assertEqual(result.returncode, 1)
        self.assertIn("NOTICE", result.stderr)

    def test_musl_library_archive_holds_the_static_library_and_a_static_pkg_config_file(self):
        static_library = self.root / "libnurt_ffi.a"
        static_library.write_bytes(b"static library")
        for target in ("x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"):
            result = self.pack(target, self.root / "out", "--library", str(static_library))
            self.assertEqual(result.returncode, 0, result.stderr)
            top = f"nurt-lib-0.1.0-{target}"
            with tarfile.open(self.root / "out" / f"{top}.tar.gz") as archive:
                names = {member.name for member in archive.getmembers() if member.isfile()}
                library = archive.extractfile(f"{top}/libnurt.a").read()
                pc = archive.extractfile(f"{top}/nurt.pc").read().decode()
            self.assertEqual(names, {f"{top}/{name}" for name in ("libnurt.a", "nurt.h", "nurt.pc", "LICENSE", "NOTICE")})
            self.assertEqual(library, b"static library")
            self.assertIn("\nLibs: -L${libdir} -lnurt\n", pc)
            private = [line for line in pc.splitlines() if line.startswith("Libs.private:")]
            self.assertEqual(len(private), 1, pc)
            for flag in ("-lunwind", "-lc"):
                self.assertIn(flag, private[0].split())

    def test_glibc_pkg_config_file_has_no_private_libraries(self):
        result = self.pack("x86_64-unknown-linux-gnu", self.root / "out", "--library", str(self.library))
        self.assertEqual(result.returncode, 0, result.stderr)
        top = "nurt-lib-0.1.0-x86_64-unknown-linux-gnu"
        with tarfile.open(self.root / "out" / f"{top}.tar.gz") as archive:
            self.assertNotIn("Libs.private", archive.extractfile(f"{top}/nurt.pc").read().decode())

    def test_library_is_rejected_for_unsupported_targets(self):
        result = self.pack("x86_64-unknown-freebsd", self.root / "out", "--library", str(self.library))
        self.assertEqual(result.returncode, 1)
        self.assertIn("Linux (glibc and musl), macOS and Windows targets only", result.stderr)

    def test_library_is_named_after_the_platform_of_the_target(self):
        for target, library in (
            ("aarch64-apple-darwin", "libnurt.1.dylib"),
            ("x86_64-apple-darwin", "libnurt.1.dylib"),
        ):
            result = self.pack(target, self.root / "out", "--library", str(self.library))
            self.assertEqual(result.returncode, 0, result.stderr)
            top = f"nurt-lib-0.1.0-{target}"
            names = self.names(self.root / "out" / f"{top}.tar.gz")
            self.assertIn(f"{top}/{library}", names)
            self.assertIn(f"{top}/nurt.h", names)
            self.assertIn(f"{top}/nurt.pc", names)
            self.assertNotIn(f"nurt-0.1.0-{target}/{library}", self.names(self.root / "out" / f"nurt-0.1.0-{target}.tar.gz"))
        target = "x86_64-pc-windows-msvc"
        result = self.pack(target, self.root / "out", "--library", str(self.library))
        self.assertEqual(result.returncode, 0, result.stderr)
        names = self.names(self.root / "out" / f"nurt-lib-0.1.0-{target}.zip")
        self.assertIn("nurt.dll", names)
        self.assertIn("nurt.h", names)
        self.assertNotIn("nurt.pc", names)

    def test_library_archives_are_reproducible(self):
        target = "x86_64-unknown-linux-gnu"
        first, second = self.root / "first", self.root / "second"
        self.assertEqual(self.pack(target, first, "--library", str(self.library)).returncode, 0)
        self.assertEqual(self.pack(target, second, "--library", str(self.library)).returncode, 0)
        name = f"nurt-lib-0.1.0-{target}.tar.gz"
        self.assertEqual((first / name).read_bytes(), (second / name).read_bytes())

    def test_rejects_a_missing_library_file(self):
        self.library.unlink()
        result = self.pack("x86_64-unknown-linux-gnu", self.root / "out", "--library", str(self.library))
        self.assertEqual(result.returncode, 1)
        self.assertIn("libnurt.so.1", result.stderr)

    def test_zip_entries_do_not_depend_on_the_platform_that_packed_them(self):
        target = "x86_64-pc-windows-msvc"
        self.assertEqual(self.pack(target, self.root / "out").returncode, 0)
        with zipfile.ZipFile(self.root / "out" / f"nurt-0.1.0-{target}.zip") as archive:
            self.assertEqual({info.create_system for info in archive.infolist()}, {3})

    def test_rejects_a_source_date_epoch_that_is_not_a_number(self):
        for value in ("yesterday", "-5", ""):
            environment = dict(os.environ, SOURCE_DATE_EPOCH=value)
            result = subprocess.run(
                [sys.executable, "-I", str(SCRIPT), "--root", str(self.repository),
                 "--version", "0.1.0", "--target", "x86_64-unknown-linux-gnu",
                 "--binary", str(self.binary), "--output", str(self.root / "out"),
                 "--completions", str(self.completions), "--site", str(self.site)],
                capture_output=True, text=True, check=False, env=environment,
            )
            self.assertEqual(result.returncode, 1, value)
            self.assertIn("SOURCE_DATE_EPOCH", result.stderr)
            self.assertNotIn("Traceback", result.stderr)

    def test_rejects_an_invalid_version(self):
        result = subprocess.run(
            [sys.executable, "-I", str(SCRIPT), "--root", str(self.repository), "--version", "1.0",
             "--target", "x86_64-unknown-linux-gnu", "--binary", str(self.binary),
             "--output", str(self.root / "out")],
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("invalid version", result.stderr)


if __name__ == "__main__":
    unittest.main()
