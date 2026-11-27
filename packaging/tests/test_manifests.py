# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Structural checks of the rendered package manifests.

The official schemas of winget and Scoop are applied in the CI workflow; the
checks here cover the fields and formats that the schemas require, without
network access.
"""

import json
import re
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ElementTree
from pathlib import Path

import support
from test_render import checksums_text

VERSION = "1.4.0"
EPOCH = "1798000000"
SHA256 = re.compile(r"^[0-9a-f]{64}$")
URL = re.compile(r"^https://github\.com/DanacoOfficial/danaco-nurt/releases/download/v[^/]+/nurt-[^/]+\.(zip|tar\.gz)$")


class RenderedManifestsTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        root = Path(cls.temp.name)
        checksums = root / "checksums.txt"
        checksums.write_text(checksums_text(VERSION, digest="0123456789abcdef" * 4), encoding="utf-8")
        cls.out = root / "out"
        subprocess.run(
            ["python3", "-I", str(support.PACKAGING / "release" / "render.py"),
             "--checksums", str(checksums), "--output", str(cls.out), "--epoch", EPOCH],
            check=True, capture_output=True,
        )

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def read(self, relative):
        return (self.out / relative).read_text(encoding="utf-8")

    def winget(self, name):
        return self.read(f"winget/manifests/d/Danaco/Nurt/{VERSION}/{name}")

    def scalar(self, text, key):
        match = re.search(rf"^{key}: (.+)$", text, re.MULTILINE)
        self.assertIsNotNone(match, key)
        return match.group(1)

    def test_scoop_manifest_has_required_fields(self):
        manifest = json.loads(self.read("scoop/nurt.json"))
        self.assertEqual(manifest["version"], VERSION)
        for key in ("description", "homepage", "license", "architecture", "bin", "checkver", "autoupdate"):
            self.assertIn(key, manifest)
        for architecture in ("64bit", "arm64"):
            entry = manifest["architecture"][architecture]
            self.assertRegex(entry["url"], URL)
            self.assertRegex(entry["hash"], SHA256)
            self.assertIn("$version", manifest["autoupdate"]["architecture"][architecture]["url"])

    def test_scoop_urls_name_the_matching_archive(self):
        manifest = json.loads(self.read("scoop/nurt.json"))
        self.assertTrue(manifest["architecture"]["64bit"]["url"].endswith(f"nurt-{VERSION}-x86_64-pc-windows-msvc.zip"))
        self.assertTrue(manifest["architecture"]["arm64"]["url"].endswith(f"nurt-{VERSION}-aarch64-pc-windows-msvc.zip"))

    def test_winget_manifests_share_identifier_and_version(self):
        for name in ("Danaco.Nurt.yaml", "Danaco.Nurt.installer.yaml", "Danaco.Nurt.locale.en-US.yaml"):
            text = self.winget(name)
            self.assertEqual(self.scalar(text, "PackageIdentifier"), "Danaco.Nurt")
            self.assertEqual(self.scalar(text, "PackageVersion"), VERSION)
            self.assertEqual(self.scalar(text, "ManifestVersion"), "1.6.0")

    def test_winget_manifest_types(self):
        self.assertEqual(self.scalar(self.winget("Danaco.Nurt.yaml"), "ManifestType"), "version")
        self.assertEqual(self.scalar(self.winget("Danaco.Nurt.installer.yaml"), "ManifestType"), "installer")
        self.assertEqual(self.scalar(self.winget("Danaco.Nurt.locale.en-US.yaml"), "ManifestType"), "defaultLocale")
        self.assertEqual(self.scalar(self.winget("Danaco.Nurt.yaml"), "DefaultLocale"), "en-US")

    def test_winget_installer_is_a_portable_zip_for_two_architectures(self):
        text = self.winget("Danaco.Nurt.installer.yaml")
        self.assertEqual(self.scalar(text, "InstallerType"), "zip")
        self.assertEqual(self.scalar(text, "NestedInstallerType"), "portable")
        self.assertIn("- RelativeFilePath: nurt.exe", text)
        self.assertEqual(re.findall(r"^- Architecture: (\w+)$", text, re.MULTILINE), ["x64", "arm64"])
        for url in re.findall(r"InstallerUrl: (\S+)", text):
            self.assertRegex(url, URL)
        digests = re.findall(r"InstallerSha256: (\S+)", text)
        self.assertEqual(len(digests), 2)
        for digest in digests:
            self.assertRegex(digest, SHA256)
        self.assertRegex(self.scalar(text, "ReleaseDate"), r"^\d{4}-\d{2}-\d{2}$")

    def test_winget_locale_has_required_fields(self):
        text = self.winget("Danaco.Nurt.locale.en-US.yaml")
        for key in ("PackageLocale", "Publisher", "PackageName", "License", "ShortDescription"):
            self.assertTrue(self.scalar(text, key))
        self.assertLessEqual(len(self.scalar(text, "ShortDescription")), 256)

    def test_chocolatey_nuspec_is_valid_xml_with_required_metadata(self):
        namespace = "{http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd}"
        metadata = ElementTree.fromstring(self.read("chocolatey/nurt.nuspec")).find(f"{namespace}metadata")
        values = {child.tag.replace(namespace, ""): child.text for child in metadata}
        self.assertEqual(values["id"], "nurt")
        self.assertEqual(values["version"], VERSION)
        for key in ("authors", "description", "projectUrl", "licenseUrl", "tags"):
            self.assertTrue(values[key], key)

    def test_chocolatey_install_script_verifies_checksums(self):
        text = self.read("chocolatey/tools/chocolateyinstall.ps1")
        self.assertEqual(len(re.findall(r"\$checksum = '[0-9a-f]{64}'", text)), 2)
        self.assertIn("ChecksumType64 = 'sha256'", text)
        self.assertIn(f"nurt-{VERSION}-x86_64-pc-windows-msvc.zip", text)

    def test_homebrew_formula_names_four_archives_with_digests(self):
        text = self.read("homebrew/nurt.rb")
        self.assertEqual(len(re.findall(r'sha256 "[0-9a-f]{64}"', text)), 4)
        self.assertEqual(len(re.findall(r'url "https://github\.com/DanacoOfficial/danaco-nurt/releases/download/', text)), 4)
        self.assertIn('assert_match(/\\Anurt \\d/, shell_output("#{bin}/nurt version"))', text)

    def test_homebrew_formula_takes_the_glibc_archives_of_linux(self):
        text = self.read("homebrew/nurt.rb")
        for target in ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"):
            self.assertIn(f"nurt-{VERSION}-{target}.tar.gz", text)
        self.assertNotIn("musl", text)

    def test_homebrew_core_formula_builds_from_source_archive(self):
        text = self.read("homebrew/core/nurt.rb")
        self.assertIn(f'url "https://github.com/DanacoOfficial/danaco-nurt/archive/refs/tags/v{VERSION}.tar.gz"', text)
        self.assertRegex(text, r'sha256 "[0-9a-f]{64}"')
        self.assertIn('depends_on "rust" => :build', text)

    def test_rpm_spec_carries_version_release_and_changelog_date(self):
        text = self.read("rpm/nurt.spec")
        self.assertRegex(text, rf"(?m)^Version:\s+{re.escape(VERSION)}$")
        self.assertRegex(text, r"(?m)^Release:\s+1%\{\?dist\}$")
        self.assertIn("* Wed Dec 23 2026 ", text)
        for package in ("%package -n libnurt\n", "%package -n libnurt-devel\n"):
            self.assertIn(package, text)
        self.assertRegex(text, r"(?m)^ExclusiveArch:\s+x86_64 aarch64$")

    def test_rpm_spec_generates_the_requirements_of_the_linked_libraries(self):
        text = self.read("rpm/nurt.spec")
        self.assertNotRegex(text, r"(?m)^AutoReq(Prov)?:\s+no\b")
        self.assertRegex(text, r"(?m)^AutoProv:\s+no$")


# Product description required in every package manifest and package manager
# listing: the constant TAGLINE of nurt_contracts::product. The suite holds no
# literal of it.
PRODUCT_DESCRIPTION = support.product_tagline()


class ProductDescriptionTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        root = Path(cls.temp.name)
        checksums = root / "checksums.txt"
        checksums.write_text(checksums_text(VERSION, digest="0123456789abcdef" * 4), encoding="utf-8")
        cls.out = root / "out"
        subprocess.run(
            ["python3", "-I", str(support.PACKAGING / "release" / "render.py"),
             "--checksums", str(checksums), "--output", str(cls.out), "--epoch", EPOCH],
            check=True, capture_output=True,
        )
        cls.deb_script = (support.PACKAGING / "deb" / "build-deb.sh").read_text(encoding="utf-8")

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def read(self, relative):
        return (self.out / relative).read_text(encoding="utf-8")

    def test_description_is_ascii_without_trailing_period(self):
        self.assertTrue(PRODUCT_DESCRIPTION.isascii())
        self.assertFalse(PRODUCT_DESCRIPTION.endswith("."))

    def test_deb_summary_uses_product_description(self):
        self.assertIn("summary=$(read_tagline)\n", self.deb_script)
        self.assertNotIn(PRODUCT_DESCRIPTION, self.deb_script)
        self.assertIn('"Description: $summary" \\\n', self.deb_script)

    def test_rpm_summary_uses_product_description(self):
        self.assertRegex(self.read("rpm/nurt.spec"), rf"(?m)^Summary:\s+{re.escape(PRODUCT_DESCRIPTION)}$")

    def test_homebrew_formulas_use_product_description(self):
        for name in ("homebrew/nurt.rb", "homebrew/core/nurt.rb"):
            self.assertIn(f'desc "{PRODUCT_DESCRIPTION}"\n', self.read(name))

    def test_winget_short_description_uses_product_description(self):
        text = self.read(f"winget/manifests/d/Danaco/Nurt/{VERSION}/Danaco.Nurt.locale.en-US.yaml")
        self.assertRegex(text, rf"(?m)^ShortDescription: {re.escape(PRODUCT_DESCRIPTION)}$")

    def test_scoop_description_uses_product_description(self):
        self.assertEqual(json.loads(self.read("scoop/nurt.json"))["description"], PRODUCT_DESCRIPTION)

    def test_chocolatey_summary_uses_product_description(self):
        root = ElementTree.fromstring(self.read("chocolatey/nurt.nuspec"))
        summary = root.find(".//{*}summary")
        self.assertIsNotNone(summary)
        self.assertEqual(summary.text, PRODUCT_DESCRIPTION)

    def test_templates_do_not_carry_retired_wording(self):
        retired = ("command console", "for people and AI agents", "for AI agents")
        for path in support.PACKAGING.rglob("*"):
            if not path.is_file() or "tests" in path.relative_to(support.PACKAGING).parts:
                continue
            try:
                text = path.read_text(encoding="utf-8").lower()
            except UnicodeDecodeError:
                continue
            for phrase in retired:
                self.assertNotIn(phrase.lower(), text, str(path))


if __name__ == "__main__":
    unittest.main()
