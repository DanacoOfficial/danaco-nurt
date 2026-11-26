# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Tests of packaging/deb/build-deb.sh and packaging/apt-repo/mkrepo.py."""

import gzip
import hashlib
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import support

BUILD = support.PACKAGING / "deb" / "build-deb.sh"
EPOCH = "1798000000"


@unittest.skipIf(shutil.which("dpkg-deb") is None, "dpkg-deb is not installed")
class DebBuildTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.artifact = support.make_artifact(self.root / "artifact")

    def build(self, output="out", version=support.VERSION, arch="amd64", artifact=None):
        result = subprocess.run(
            [
                "sh", str(BUILD),
                "--artifact", str(artifact or self.artifact),
                "--version", version,
                "--arch", arch,
                "--output", str(self.root / output),
            ],
            capture_output=True,
            text=True,
            env=support.clean_environment(SOURCE_DATE_EPOCH=EPOCH),
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return [Path(line) for line in result.stdout.split()]

    def field(self, package, name):
        result = subprocess.run(
            ["dpkg-deb", "--field", str(package), name], capture_output=True, text=True, check=True
        )
        return result.stdout.strip()

    def contents(self, package):
        result = subprocess.run(["dpkg-deb", "--contents", str(package)], capture_output=True, text=True, check=True)
        return result.stdout

    def test_builds_three_packages_with_expected_names(self):
        names = sorted(path.name for path in self.build())
        self.assertEqual(
            names,
            [
                f"libnurt-dev_{support.VERSION}_amd64.deb",
                f"libnurt1_{support.VERSION}_amd64.deb",
                f"nurt_{support.VERSION}_amd64.deb",
            ],
        )

    def test_control_fields_of_the_command_package(self):
        nurt = next(p for p in self.build() if p.name.startswith("nurt_"))
        self.assertEqual(self.field(nurt, "Package"), "nurt")
        self.assertEqual(self.field(nurt, "Architecture"), "amd64")
        self.assertEqual(self.field(nurt, "Maintainer"), "Danaco Official <support@danaco-group.pl>")
        self.assertEqual(self.field(nurt, "Section"), "utils")

    def test_command_package_depends_on_the_shared_libraries_it_links(self):
        nurt = next(p for p in self.build() if p.name.startswith("nurt_"))
        depends = self.field(nurt, "Depends")
        for library in ("libc6", "libgcc-s1"):
            self.assertIn(library, depends)
        self.assertNotIn("liblzma", depends)

    def test_command_package_description_is_the_product_tagline(self):
        nurt = next(p for p in self.build() if p.name.startswith("nurt_"))
        self.assertEqual(self.field(nurt, "Description").splitlines()[0], support.product_tagline())

    def test_command_package_contents(self):
        nurt = next(p for p in self.build() if p.name.startswith("nurt_"))
        listing = self.contents(nurt)
        for expected in (
            "./usr/bin/nurt",
            "./usr/share/man/man1/nurt.1.gz",
            "./usr/share/bash-completion/completions/nurt",
            "./usr/share/zsh/vendor-completions/_nurt",
            "./usr/share/fish/vendor_completions.d/nurt.fish",
            "./usr/share/nurt/service/nurt.service",
            "./usr/share/doc/nurt/copyright",
            "./usr/share/doc/nurt/changelog.gz",
        ):
            self.assertIn(expected, listing)

    def test_library_packages_use_multiarch_directories(self):
        packages = {p.name.split("_")[0]: p for p in self.build(arch="arm64")}
        self.assertIn("./usr/lib/aarch64-linux-gnu/libnurt.so.1", self.contents(packages["libnurt1"]))
        dev = self.contents(packages["libnurt-dev"])
        self.assertIn("./usr/include/nurt.h", dev)
        self.assertIn("./usr/lib/aarch64-linux-gnu/libnurt.so -> libnurt.so.1", dev)
        self.assertIn("./usr/lib/aarch64-linux-gnu/pkgconfig/nurt.pc", dev)
        self.assertEqual(self.field(packages["libnurt-dev"], "Depends"), f"libnurt1 (= {support.VERSION})")

    def test_pkg_config_file_is_rewritten_for_the_system_prefix(self):
        dev = next(p for p in self.build() if p.name.startswith("libnurt-dev"))
        extract = self.root / "extract"
        subprocess.run(["dpkg-deb", "--extract", str(dev), str(extract)], check=True)
        text = (extract / "usr/lib/x86_64-linux-gnu/pkgconfig/nurt.pc").read_text(encoding="utf-8")
        self.assertIn("prefix=/usr\n", text)
        self.assertIn("libdir=${prefix}/lib/x86_64-linux-gnu\n", text)
        self.assertNotIn("/opt/example", text)

    def test_library_package_declares_shlibs_and_ldconfig_trigger(self):
        lib = next(p for p in self.build() if p.name.startswith("libnurt1"))
        control = self.root / "control"
        subprocess.run(["dpkg-deb", "--control", str(lib), str(control)], check=True)
        self.assertEqual(
            (control / "shlibs").read_text(encoding="utf-8"), f"libnurt 1 libnurt1 (>= {support.VERSION})\n"
        )
        self.assertEqual((control / "triggers").read_text(encoding="utf-8"), "activate-noawait ldconfig\n")

    def test_pre_release_version_uses_tilde(self):
        nurt = next(p for p in self.build(version="1.0.0-rc.1") if p.name.startswith("nurt_"))
        self.assertEqual(self.field(nurt, "Version"), "1.0.0~rc.1")

    def test_build_is_reproducible(self):
        first = self.build("first")
        second = self.build("second")
        for one, two in zip(first, second):
            self.assertEqual(
                hashlib.sha256(one.read_bytes()).hexdigest(),
                hashlib.sha256(two.read_bytes()).hexdigest(),
                one.name,
            )

    def test_accepts_archive_with_top_directory(self):
        archive = self.root / "nurt.tar.gz"
        subprocess.run(
            ["tar", "-czf", str(archive), "-C", str(self.root), "--transform", "s|^artifact|nurt-top|", "artifact"],
            check=True,
        )
        names = [path.name for path in self.build(artifact=archive)]
        self.assertEqual(len(names), 3)

    def test_rejects_artifact_without_library(self):
        (self.artifact / "libnurt.so.1").unlink()
        result = subprocess.run(
            ["sh", str(BUILD), "--artifact", str(self.artifact), "--version", support.VERSION,
             "--arch", "amd64", "--output", str(self.root / "out")],
            capture_output=True, text=True, env=support.clean_environment(SOURCE_DATE_EPOCH=EPOCH), check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("lacks libnurt.so.1", result.stderr)

    def test_changelog_names_the_release(self):
        nurt = next(p for p in self.build() if p.name.startswith("nurt_"))
        extract = self.root / "extract"
        subprocess.run(["dpkg-deb", "--extract", str(nurt), str(extract)], check=True)
        text = gzip.decompress((extract / "usr/share/doc/nurt/changelog.gz").read_bytes()).decode("utf-8")
        self.assertTrue(text.startswith(f"nurt ({support.VERSION}) stable; urgency=medium"))
        self.assertIn("Wed, 23 Dec 2026", text)


@unittest.skipIf(shutil.which("dpkg-deb") is None, "dpkg-deb is not installed")
class AptRepositoryTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        artifact = support.make_artifact(self.root / "artifact")
        self.packages = self.root / "packages"
        for arch in ("amd64", "arm64"):
            subprocess.run(
                ["sh", str(BUILD), "--artifact", str(artifact), "--version", support.VERSION,
                 "--arch", arch, "--output", str(self.packages)],
                env=support.clean_environment(SOURCE_DATE_EPOCH=EPOCH),
                capture_output=True, check=True,
            )
        self.repository = self.root / "repo"

    def generate(self, *extra):
        return subprocess.run(
            ["python3", "-I", str(support.PACKAGING / "apt-repo" / "mkrepo.py"),
             "--packages", str(self.packages), "--output", str(self.repository), *extra],
            capture_output=True, text=True, env=support.clean_environment(SOURCE_DATE_EPOCH=EPOCH), check=False,
        )

    def test_generates_pool_and_indices(self):
        result = self.generate()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.repository / "pool/main/n/nurt" / f"nurt_{support.VERSION}_amd64.deb").is_file())
        self.assertTrue((self.repository / "pool/main/n/nurt" / f"libnurt1_{support.VERSION}_arm64.deb").is_file())
        for arch in ("amd64", "arm64"):
            packages = (self.repository / "dists/stable/main" / f"binary-{arch}" / "Packages").read_text(encoding="utf-8")
            self.assertEqual(packages.count("Package: "), 3)
            self.assertIn(f"Filename: pool/main/n/nurt/nurt_{support.VERSION}_{arch}.deb", packages)

    def test_release_lists_matching_digests(self):
        self.assertEqual(self.generate().returncode, 0)
        suite = self.repository / "dists/stable"
        release = (suite / "Release").read_text(encoding="utf-8")
        self.assertIn("Suite: stable\n", release)
        self.assertIn("Architectures: amd64 arm64\n", release)
        self.assertIn("Date: Wed, 23 Dec 2026", release)
        listed = 0
        for line in release.split("SHA256:\n", 1)[1].splitlines():
            digest, size, name = line.split()
            data = (suite / name).read_bytes()
            self.assertEqual(hashlib.sha256(data).hexdigest(), digest)
            self.assertEqual(len(data), int(size))
            listed += 1
        self.assertEqual(listed, 6)

    def test_package_digest_matches_pool_file(self):
        self.assertEqual(self.generate().returncode, 0)
        packages = (self.repository / "dists/stable/main/binary-amd64/Packages").read_text(encoding="utf-8")
        deb = self.repository / "pool/main/n/nurt" / f"nurt_{support.VERSION}_amd64.deb"
        self.assertIn(f"SHA256: {hashlib.sha256(deb.read_bytes()).hexdigest()}", packages)

    def test_index_is_reproducible(self):
        self.assertEqual(self.generate().returncode, 0)
        first = (self.repository / "dists/stable/main/binary-amd64/Packages.gz").read_bytes()
        shutil.rmtree(self.repository)
        self.assertEqual(self.generate().returncode, 0)
        self.assertEqual(first, (self.repository / "dists/stable/main/binary-amd64/Packages.gz").read_bytes())

    def test_unsigned_repository_has_no_signature_files(self):
        self.assertEqual(self.generate().returncode, 0)
        self.assertFalse((self.repository / "dists/stable/InRelease").exists())

    @unittest.skipIf(shutil.which("gpg") is not None, "gpg is installed")
    def test_signing_without_gpg_fails(self):
        result = self.generate("--sign-key", "example")
        self.assertEqual(result.returncode, 1)
        self.assertIn("gpg is not installed", result.stderr)

    def test_rejects_a_source_date_epoch_that_is_not_a_number(self):
        result = subprocess.run(
            ["python3", "-I", str(support.PACKAGING / "apt-repo" / "mkrepo.py"),
             "--packages", str(self.packages), "--output", str(self.repository)],
            capture_output=True, text=True, check=False,
            env=support.clean_environment(SOURCE_DATE_EPOCH="soon"),
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("SOURCE_DATE_EPOCH", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_fails_for_directory_without_packages(self):
        empty = self.root / "empty"
        empty.mkdir()
        result = subprocess.run(
            ["python3", "-I", str(support.PACKAGING / "apt-repo" / "mkrepo.py"),
             "--packages", str(empty), "--output", str(self.repository)],
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("no .deb files", result.stderr)


if __name__ == "__main__":
    unittest.main()
