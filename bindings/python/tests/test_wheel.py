# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Platform wheel built from the mocks and installed into a scratch directory."""

from __future__ import annotations

import base64
import hashlib
import importlib.util
import os
import platform
import shutil
import subprocess
import sys
import unittest
import zipfile
from pathlib import Path
from typing import Any, Dict

import support

HOST_TARGETS = {
    ("linux", "x86_64"): "x86_64-unknown-linux-gnu",
    ("linux", "aarch64"): "aarch64-unknown-linux-gnu",
    ("darwin", "x86_64"): "x86_64-apple-darwin",
    ("darwin", "arm64"): "aarch64-apple-darwin",
}


def load_builder() -> Any:
    spec = importlib.util.spec_from_file_location("build_wheels", support.PACKAGE_DIR / "tools" / "build_wheels.py")
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@unittest.skipIf(sys.version_info < (3, 11), "the wheel builder needs tomllib")
class WheelTest(unittest.TestCase):
    wheel: Path
    site: Path

    @classmethod
    def setUpClass(cls) -> None:
        target = HOST_TARGETS.get((sys.platform, platform.machine()))
        if target is None:
            raise unittest.SkipTest("no wheel target for this host")
        if not (support.REPOSITORY_DIR / "LICENSE").is_file():
            raise unittest.SkipTest("LICENSE is not available outside the repository")
        root = support.scratch() / "wheel"
        artifacts = root / "artifacts" / target
        support.write_launcher(artifacts)
        shutil.copy(support.mock_library(), artifacts / Path(support.mock_library()).name)
        builder = load_builder()
        cls.wheel = builder.build(target, artifacts, root / "dist", builder.package_version(), 0)
        cls.site = root / "site"
        with zipfile.ZipFile(cls.wheel) as archive:
            archive.extractall(cls.site)
            for info in archive.infolist():
                mode = info.external_attr >> 16
                if mode & 0o111:
                    (cls.site / info.filename).chmod(mode & 0o777)

    def test_file_name_carries_platform_tags(self) -> None:
        self.assertRegex(self.wheel.name, r"^nurt-\d+\.\d+\.\d+-py3-none-[a-z0-9_.]+\.whl$")

    def test_record_lists_every_file_with_its_hash(self) -> None:
        with zipfile.ZipFile(self.wheel) as archive:
            names = archive.namelist()
            record_name = next(n for n in names if n.endswith(".dist-info/RECORD"))
            rows = [line.split(",") for line in archive.read(record_name).decode().splitlines()]
            self.assertEqual(sorted(r[0] for r in rows), sorted(names))
            for name, digest, size in rows:
                if name == record_name:
                    continue
                data = archive.read(name)
                expected = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
                self.assertEqual(digest, f"sha256={expected}", name)
                self.assertEqual(int(size), len(data), name)

    def test_metadata_and_entry_point(self) -> None:
        with zipfile.ZipFile(self.wheel) as archive:
            names = archive.namelist()
            meta = archive.read(next(n for n in names if n.endswith("/METADATA"))).decode()
            wheel = archive.read(next(n for n in names if n.endswith("/WHEEL"))).decode()
            entry = archive.read(next(n for n in names if n.endswith("/entry_points.txt"))).decode()
        self.assertIn("Name: nurt\n", meta)
        self.assertIn("License-Expression: Apache-2.0\n", meta)
        self.assertIn("Requires-Python: >=3.9\n", meta)
        self.assertIn("Root-Is-Purelib: false\n", wheel)
        self.assertIn("nurt = nurt._cli:main", entry)
        self.assertIn("nurt/py.typed", names)
        self.assertTrue(any(n.endswith(".dist-info/licenses/NOTICE") for n in names))

    def run_installed(self, code: str) -> str:
        environment: Dict[str, str] = {k: v for k, v in os.environ.items() if not k.startswith("NURT_")}
        environment["PYTHONPATH"] = str(self.site)
        result = subprocess.run(
            [sys.executable, "-c", code], capture_output=True, text=True, env=environment, timeout=60
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def test_installed_package_uses_bundled_library(self) -> None:
        output = self.run_installed(
            "from nurt import Nurt\n"
            "with Nurt() as n:\n"
            "    print(n.backend, n.run('bundled').text)\n"
        )
        self.assertEqual(output, "library bundled")

    def test_installed_package_uses_bundled_executable(self) -> None:
        output = self.run_installed(
            "from nurt import Nurt\n"
            "with Nurt(backend='process') as n:\n"
            "    print(n.backend, n.run('bundled').text)\n"
        )
        self.assertEqual(output, "process bundled")

    def test_console_script_runs_bundled_executable(self) -> None:
        output = self.run_installed("import sys\nsys.argv = ['nurt', '--version']\nfrom nurt._cli import main\nmain()\n")
        self.assertEqual(output, "nurt 0.1.0")


if __name__ == "__main__":
    unittest.main()
