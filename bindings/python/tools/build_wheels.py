# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bindings/python/tools/build_wheels.py
# Responsibility:   Script build_wheels.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Build the platform wheels of the ``nurt`` package from release artifacts.

Usage::

    python3 tools/build_wheels.py --artifacts DIR --out DIR [--target TRIPLE ...]

``DIR/<target>/`` holds the files of one target: the executable ``nurt``
(``nurt.exe`` on Windows) and, when the target has one, the shared library
(``libnurt.so.1``, ``libnurt.1.dylib`` or ``nurt.dll``). Downloaded artifacts
are verified against the published SHA-256 sums before this tool runs. The
wheel ``nurt-<version>-py3-none-<platform>.whl`` contains the Python package,
the files of the target in ``nurt/_bin`` and the license files. Archive
entries are sorted and dated by ``SOURCE_DATE_EPOCH`` so that the output is
reproducible. Requires Python 3.11 or later (``tomllib``).
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import os
import re
import stat
import sys
import time
import zipfile
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Tuple

PACKAGE_DIR = Path(__file__).resolve().parent.parent
SOURCE_DIR = PACKAGE_DIR / "src" / "nurt"
REPOSITORY_DIR = PACKAGE_DIR.parent.parent
LICENSE_FILES = ("LICENSE", "NOTICE")
GENERATOR = "nurt-build-wheels"
ZIP_EPOCH = 315532800
"""1980-01-01T00:00:00Z, the earliest time a ZIP entry can record."""

TARGETS: Dict[str, Tuple[str, ...]] = {
    "x86_64-unknown-linux-gnu": ("manylinux_2_17_x86_64", "manylinux2014_x86_64"),
    "aarch64-unknown-linux-gnu": ("manylinux_2_17_aarch64", "manylinux2014_aarch64"),
    "x86_64-unknown-linux-musl": ("musllinux_1_2_x86_64",),
    "aarch64-unknown-linux-musl": ("musllinux_1_2_aarch64",),
    "x86_64-apple-darwin": ("macosx_11_0_x86_64",),
    "aarch64-apple-darwin": ("macosx_11_0_arm64",),
    "x86_64-pc-windows-msvc": ("win_amd64",),
    "aarch64-pc-windows-msvc": ("win_arm64",),
}
"""Wheel platform tags of each Rust target."""


def executable_name(target: str) -> str:
    return "nurt.exe" if "windows" in target else "nurt"


def library_name(target: str) -> str:
    if "windows" in target:
        return "nurt.dll"
    if "apple" in target:
        return "libnurt.1.dylib"
    return "libnurt.so.1"


def package_version() -> str:
    text = (SOURCE_DIR / "_version.py").read_text(encoding="utf-8")
    match = re.search(r'^__version__ = "([^"]+)"', text, re.M)
    if match is None:
        raise SystemExit("src/nurt/_version.py does not define __version__")
    return match.group(1)


def metadata(version: str) -> str:
    """Core metadata 2.4 of the wheel from ``pyproject.toml``."""
    import tomllib

    project = tomllib.loads((PACKAGE_DIR / "pyproject.toml").read_text(encoding="utf-8"))["project"]
    lines = [
        "Metadata-Version: 2.4",
        f"Name: {project['name']}",
        f"Version: {version}",
        f"Summary: {project['description']}",
    ]
    for author in project.get("authors", []):
        lines.append(f"Author-email: {author['name']} <{author['email']}>")
    lines.append(f"License-Expression: {project['license']}")
    lines.extend(f"License-File: {name}" for name in LICENSE_FILES)
    for label, url in project.get("urls", {}).items():
        lines.append(f"Project-URL: {label}, {url}")
    if project.get("keywords"):
        lines.append(f"Keywords: {','.join(project['keywords'])}")
    lines.extend(f"Classifier: {classifier}" for classifier in project.get("classifiers", []))
    lines.append(f"Requires-Python: {project['requires-python']}")
    lines.append("Description-Content-Type: text/markdown")
    readme = (PACKAGE_DIR / project["readme"]).read_text(encoding="utf-8")
    return "\n".join(lines) + "\n\n" + readme


def record_hash(data: bytes) -> str:
    digest = hashlib.sha256(data).digest()
    return "sha256=" + base64.urlsafe_b64encode(digest).rstrip(b"=").decode("ascii")


class WheelWriter:
    def __init__(self, path: Path, date: Tuple[int, int, int, int, int, int]) -> None:
        self.archive = zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED)
        self.date = date
        self.records: List[str] = []

    def add(self, name: str, data: bytes, executable: bool = False) -> None:
        info = zipfile.ZipInfo(name, date_time=self.date)
        mode = 0o755 if executable else 0o644
        info.external_attr = (stat.S_IFREG | mode) << 16
        info.compress_type = zipfile.ZIP_DEFLATED
        self.archive.writestr(info, data)
        self.records.append(f"{name},{record_hash(data)},{len(data)}")

    def close(self, record_name: str) -> None:
        self.records.append(f"{record_name},,")
        info = zipfile.ZipInfo(record_name, date_time=self.date)
        info.external_attr = (stat.S_IFREG | 0o644) << 16
        info.compress_type = zipfile.ZIP_DEFLATED
        self.archive.writestr(info, "\n".join(self.records) + "\n")
        self.archive.close()


def package_files() -> List[Path]:
    return sorted(
        path
        for path in SOURCE_DIR.rglob("*")
        if path.is_file()
        and "__pycache__" not in path.parts
        and "_bin" not in path.relative_to(SOURCE_DIR).parts
        and path.suffix in (".py", ".typed")
    )


def build(target: str, files_dir: Path, out_dir: Path, version: str, date_epoch: int) -> Path:
    """Write the wheel of ``target`` and return its path."""
    tags = TARGETS[target]
    executable = files_dir / executable_name(target)
    if not executable.is_file():
        raise SystemExit(f"{executable} is missing")
    library = files_dir / library_name(target)
    stamp = time.gmtime(max(date_epoch, ZIP_EPOCH))[:6]
    out_dir.mkdir(parents=True, exist_ok=True)
    wheel = out_dir / f"nurt-{version}-py3-none-{'.'.join(tags)}.whl"
    dist_info = f"nurt-{version}.dist-info"
    writer = WheelWriter(wheel, stamp)
    for path in package_files():
        writer.add(f"nurt/{path.relative_to(SOURCE_DIR).as_posix()}", path.read_bytes())
    writer.add(f"nurt/_bin/{executable.name}", executable.read_bytes(), executable=True)
    if library.is_file():
        writer.add(f"nurt/_bin/{library.name}", library.read_bytes(), executable=True)
    writer.add(f"{dist_info}/METADATA", metadata(version).encode("utf-8"))
    wheel_file = ["Wheel-Version: 1.0", f"Generator: {GENERATOR}", "Root-Is-Purelib: false"]
    wheel_file += [f"Tag: py3-none-{tag}" for tag in tags]
    writer.add(f"{dist_info}/WHEEL", ("\n".join(wheel_file) + "\n").encode("utf-8"))
    writer.add(f"{dist_info}/entry_points.txt", b"[console_scripts]\nnurt = nurt._cli:main\n")
    for name in LICENSE_FILES:
        writer.add(f"{dist_info}/licenses/{name}", (REPOSITORY_DIR / name).read_bytes())
    writer.close(f"{dist_info}/RECORD")
    return wheel


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--artifacts", type=Path, required=True, help="directory with one subdirectory per target")
    parser.add_argument("--out", type=Path, required=True, help="directory for the wheels")
    parser.add_argument("--target", action="append", choices=sorted(TARGETS), help="target to build (repeatable)")
    arguments = parser.parse_args(argv)
    targets = arguments.target or sorted(t for t in TARGETS if (arguments.artifacts / t).is_dir())
    if not targets:
        sys.stderr.write(f"no target directories in {arguments.artifacts}\n")
        return 1
    version = package_version()
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", ZIP_EPOCH))
    for target in targets:
        print(build(target, arguments.artifacts / target, arguments.out, version, epoch))
    return 0


if __name__ == "__main__":
    sys.exit(main())

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
