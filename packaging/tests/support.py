# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Shared helpers of the packaging tests.

The tests use a mock release artifact: a directory laid out like a release
archive whose nurt executable is a shell script that imitates "nurt version".
"""

import hashlib
import io
import os
import platform
import re
import stat
import tarfile
import zipfile
from pathlib import Path

PACKAGING = Path(__file__).resolve().parent.parent
VERSION = "0.1.0"
PRODUCT_SOURCE = PACKAGING.parent / "crates" / "nurt-contracts" / "src" / "product.rs"

MOCK_EXECUTABLE = """#!/bin/sh
if [ "$1" = version ]; then
    echo "nurt 0.1.0"
    exit 0
fi
exit 2
"""

MOCK_PKG_CONFIG = """prefix=/opt/example
libdir=${prefix}/lib
includedir=${prefix}/include

Name: nurt
Description: C interface of Danaco Nurt
Version: 0.1.0
Libs: -L${libdir} -lnurt
Cflags: -I${includedir}
"""

TARGETS = (
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
)


def product_tagline():
    """Return the value of the constant TAGLINE read from the product texts source."""
    match = re.search(r'pub const TAGLINE:\s*&str\s*=\s*"([^"]*)"\s*;', PRODUCT_SOURCE.read_text(encoding="utf-8"))
    if match is None:
        raise RuntimeError(f"{PRODUCT_SOURCE} does not define TAGLINE")
    return match.group(1)


def host_target():
    """Return the musl or Darwin target of the machine running the tests."""
    arch = {"amd64": "x86_64", "arm64": "aarch64"}.get(platform.machine().lower(), platform.machine())
    if platform.system() == "Darwin":
        return f"{arch}-apple-darwin"
    return f"{arch}-unknown-linux-musl"


def write_executable(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def make_artifact(directory):
    """Create the contents of a Linux release archive in directory."""
    write_executable(directory / "nurt", MOCK_EXECUTABLE)
    (directory / "libnurt.so.1").write_bytes(b"mock shared library\n")
    (directory / "nurt.h").write_text("/* mock header */\n", encoding="utf-8")
    (directory / "nurt.pc").write_text(MOCK_PKG_CONFIG, encoding="utf-8")
    (directory / "NOTICE").write_text("Danaco Nurt\n", encoding="utf-8")
    (directory / "LICENSE").write_text("mock license\n", encoding="utf-8")
    man = directory / "man" / "man1"
    man.mkdir(parents=True)
    (man / "nurt.1").write_text(".TH NURT 1\n", encoding="utf-8")
    completions = directory / "completions"
    completions.mkdir()
    (completions / "nurt.bash").write_text("complete -F _nurt nurt\n", encoding="utf-8")
    (completions / "_nurt").write_text("#compdef nurt\n", encoding="utf-8")
    (completions / "nurt.fish").write_text("complete -c nurt\n", encoding="utf-8")
    service = directory / "service"
    service.mkdir()
    (service / "nurt.service").write_text("[Service]\nExecStart=/usr/bin/nurt serve\n", encoding="utf-8")
    return directory


def tar_bytes(top, files):
    """Return a .tar.gz with files (name -> text) below the directory top."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
        for name, text in files.items():
            data = text.encode("utf-8")
            info = tarfile.TarInfo(f"{top}/{name}")
            info.size = len(data)
            info.mode = 0o755 if name == "nurt" else 0o644
            info.mtime = 0
            archive.addfile(info, io.BytesIO(data))
    return buffer.getvalue()


def zip_bytes(files):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        for name, text in files.items():
            archive.writestr(name, text)
    return buffer.getvalue()


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def make_release_tree(root, version=VERSION, targets=TARGETS, executable=MOCK_EXECUTABLE):
    """Create a release location with the layout of GitHub releases.

    Returns the mapping from archive name to its content.
    """
    download = root / "download" / f"v{version}"
    download.mkdir(parents=True)
    archives = {}
    for target in targets:
        if "windows" in target:
            name = f"nurt-{version}-{target}.zip"
            archives[name] = zip_bytes({"nurt.exe": executable, "NOTICE": "Danaco Nurt\n"})
        else:
            name = f"nurt-{version}-{target}.tar.gz"
            archives[name] = tar_bytes(
                f"nurt-{version}-{target}", {"nurt": executable, "NOTICE": "Danaco Nurt\n"}
            )
    lines = []
    for name, data in sorted(archives.items()):
        (download / name).write_bytes(data)
        lines.append(f"{sha256(data)}  {name}")
    checksums = "\n".join(lines) + "\n"
    (download / "checksums.txt").write_text(checksums, encoding="utf-8")
    latest = root / "latest" / "download"
    latest.mkdir(parents=True)
    (latest / "checksums.txt").write_text(checksums, encoding="utf-8")
    return archives


def clean_environment(**extra):
    """Return an environment without variables that influence the scripts."""
    env = {key: value for key, value in os.environ.items() if not key.startswith("NURT_")}
    env.update(extra)
    return env
