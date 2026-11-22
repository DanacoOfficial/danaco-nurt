# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Tests of packaging/release/render.py."""

import contextlib
import importlib.util
import io
import re
import tempfile
import unittest
from pathlib import Path

import support

spec = importlib.util.spec_from_file_location("render", support.PACKAGING / "release" / "render.py")
render = importlib.util.module_from_spec(spec)
spec.loader.exec_module(render)

DIGEST = "ab" * 32


def checksums_text(version, digest=DIGEST, targets=support.TARGETS, source=True):
    lines = []
    for target in targets:
        suffix = "zip" if "windows" in target else "tar.gz"
        lines.append(f"{digest}  nurt-{version}-{target}.{suffix}")
    if source:
        lines.append(f"{digest}  nurt-{version}-source.tar.gz")
    return "\n".join(lines) + "\n"


class ParseChecksumsTest(unittest.TestCase):
    def test_reads_text_and_binary_mode_entries(self):
        digests = render.parse_checksums(f"{DIGEST}  a.zip\n{DIGEST.upper()} *b.zip\n")
        self.assertEqual(digests, {"a.zip": DIGEST, "b.zip": DIGEST})

    def test_rejects_malformed_line(self):
        with self.assertRaises(render.RenderError):
            render.parse_checksums("not a digest\n")

    def test_rejects_duplicate_name(self):
        with self.assertRaises(render.RenderError):
            render.parse_checksums(f"{DIGEST}  a.zip\n{DIGEST}  a.zip\n")


class VersionTest(unittest.TestCase):
    def test_takes_version_from_archive_names(self):
        digests = render.parse_checksums(checksums_text("1.2.3-rc.1"))
        self.assertEqual(render.version_from_checksums(digests), "1.2.3-rc.1")

    def test_rejects_mixed_versions(self):
        text = checksums_text("1.0.0", targets=support.TARGETS[:1]) + checksums_text(
            "1.0.1", targets=support.TARGETS[:1], source=False
        )
        with self.assertRaises(render.RenderError):
            render.version_from_checksums(render.parse_checksums(text))

    def test_rejects_version_that_is_not_semver(self):
        with self.assertRaises(render.RenderError):
            render.parse_version("1.0")

    def test_derives_package_versions_of_a_pre_release(self):
        values = render.release_values("1.2.3-rc.1", None)
        self.assertEqual(values["DEB_VERSION"], "1.2.3~rc.1")
        self.assertEqual(values["RPM_VERSION"], "1.2.3")
        self.assertEqual(values["RPM_RELEASE"], "0.rc.1")
        self.assertEqual(values["CHOCO_VERSION"], "1.2.3-rc1")

    def test_derives_package_versions_of_a_release(self):
        values = render.release_values("1.2.3", None)
        self.assertEqual(values["DEB_VERSION"], "1.2.3")
        self.assertEqual(values["RPM_RELEASE"], "1")
        self.assertEqual(values["CHOCO_VERSION"], "1.2.3")

    def test_formats_dates_from_the_epoch(self):
        values = render.release_values("1.2.3", 1798000000)
        self.assertEqual(values["DATE"], "2026-12-23")
        self.assertEqual(values["RPM_DATE"], "Wed Dec 23 2026")


class RenderTextTest(unittest.TestCase):
    def setUp(self):
        self.digests = render.parse_checksums(checksums_text("1.0.0"))
        self.values = render.release_values("1.0.0", None)

    def render(self, text):
        return render.render_text(text, self.values, self.digests, "1.0.0", "template")

    def test_substitutes_version_and_digest(self):
        out = self.render("v@VERSION@ @SHA256:x86_64-pc-windows-msvc@")
        self.assertEqual(out, f"v1.0.0 {DIGEST}")

    def test_rejects_unknown_placeholder(self):
        with self.assertRaises(render.RenderError):
            self.render("@UNKNOWN@")

    def test_rejects_missing_archive(self):
        with self.assertRaises(render.RenderError):
            self.render("@SHA256:riscv64-unknown-linux-musl@")

    def test_rejects_date_without_epoch(self):
        with self.assertRaises(render.RenderError):
            self.render("@DATE@")

    def test_keeps_addresses_and_shell_text(self):
        text = 'mail support@danaco-group.pl "$@" @(1)'
        self.assertEqual(self.render(text), text)


class RenderTreeTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def run_main(self, *arguments):
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            code = render.main(list(arguments))
        return code, stdout.getvalue(), stderr.getvalue()

    def write_checksums(self, version="1.0.0"):
        path = self.root / "checksums.txt"
        path.write_text(checksums_text(version), encoding="utf-8")
        return path

    def test_renders_all_templates_of_the_packaging_tree(self):
        checksums = self.write_checksums("1.0.0-rc.1")
        output = self.root / "out"
        code, listing, errors = self.run_main(
            "--checksums", str(checksums), "--output", str(output), "--epoch", "1798000000"
        )
        self.assertEqual(code, 0, errors)
        rendered = sorted(listing.split())
        self.assertIn("scoop/nurt.json", rendered)
        self.assertIn("homebrew/nurt.rb", rendered)
        self.assertIn("homebrew/core/nurt.rb", rendered)
        self.assertIn("rpm/nurt.spec", rendered)
        self.assertIn("chocolatey/nurt.nuspec", rendered)
        self.assertIn("chocolatey/tools/chocolateyinstall.ps1", rendered)
        self.assertIn("winget/manifests/d/Danaco/Nurt/1.0.0-rc.1/Danaco.Nurt.installer.yaml", rendered)
        for name in rendered:
            text = (output / name).read_text(encoding="utf-8")
            self.assertIsNone(re.search(r"@[A-Z][A-Z0-9_]*(:[A-Za-z0-9_.-]+)?@", text), name)

    def test_output_is_identical_for_identical_input(self):
        checksums = self.write_checksums()
        for name in ("first", "second"):
            code, _, errors = self.run_main(
                "--checksums", str(checksums), "--output", str(self.root / name), "--epoch", "1798000000"
            )
            self.assertEqual(code, 0, errors)
        first = {p.relative_to(self.root / "first"): p.read_bytes() for p in (self.root / "first").rglob("*") if p.is_file()}
        second = {p.relative_to(self.root / "second"): p.read_bytes() for p in (self.root / "second").rglob("*") if p.is_file()}
        self.assertEqual(first, second)

    def test_fails_when_a_target_is_missing_from_checksums(self):
        path = self.root / "checksums.txt"
        path.write_text(checksums_text("1.0.0", targets=support.TARGETS[:2]), encoding="utf-8")
        code, _, errors = self.run_main(
            "--checksums", str(path), "--output", str(self.root / "out"), "--epoch", "1798000000"
        )
        self.assertEqual(code, 1)
        self.assertIn("checksums have no archive", errors)

    def test_fails_without_epoch_for_dated_templates(self):
        checksums = self.write_checksums()
        code, _, errors = self.run_main("--checksums", str(checksums), "--output", str(self.root / "out"))
        self.assertEqual(code, 1)
        self.assertIn("SOURCE_DATE_EPOCH", errors)

    def test_expands_placeholders_in_output_paths(self):
        source = self.root / "templates" / "@VERSION@"
        source.mkdir(parents=True)
        (source / "file.txt.in").write_text("@VERSION@\n", encoding="utf-8")
        checksums = self.write_checksums("2.0.0")
        code, listing, errors = self.run_main(
            "--checksums", str(checksums), "--output", str(self.root / "out"), "--source", str(self.root / "templates")
        )
        self.assertEqual(code, 0, errors)
        self.assertEqual(listing.split(), ["2.0.0/file.txt"])


if __name__ == "__main__":
    unittest.main()
