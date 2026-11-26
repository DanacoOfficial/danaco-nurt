#!/usr/bin/env python3
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Assemble the release archives of one target.

The layout is the one described in packaging/README.md. A Linux or macOS
archive is a tar.gz with the directory <name>/; a Windows archive is a zip
with the files at the top level. The archives are reproducible: the entries
are sorted, owned by root, and carry the time given with --epoch or
SOURCE_DATE_EPOCH.

The program archive nurt-<version>-<target> contains:

  nurt or nurt.exe        executable
  LICENSE, NOTICE,        texts of the repository
  README.md, CHANGELOG.md
  man/man<N>/*            manual pages (--man; not in Windows archives)
  completions/*           shell completions (--completions)
  doc/                    the documentation site (--site)

The C library goes to its own archive nurt-lib-<version>-<target>, written
when --library is given:

  libnurt.so.1,           shared library (named libnurt.so.1 on glibc Linux,
  libnurt.1.dylib or      libnurt.1.dylib on macOS and nurt.dll on Windows)
  nurt.dll
  nurt.h                  C header
  nurt.pc                 pkg-config file (not packed for Windows)
  LICENSE, NOTICE         texts of the repository

Usage:

  pack.py --root REPO --version VERSION --target TARGET --binary FILE
          --output DIR [--library FILE] [--man DIR] [--completions DIR]
          [--site DIR] [--epoch SECONDS]
"""

import argparse
import gzip
import io
import os
import re
import sys
import tarfile
import time
import zipfile
from pathlib import Path

ROOT_TEXTS = ("LICENSE", "NOTICE", "README.md", "CHANGELOG.md")
LIBRARY_TEXTS = ("LICENSE", "NOTICE")
HEADER = Path("crates") / "nurt-ffi" / "include" / "nurt.h"
SEMVER = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$")
TARGET = re.compile(r"^[a-z0-9_]+-[a-z0-9_]+-[a-z0-9_]+(?:-[a-z0-9_]+)?$")
# Zip entries record the Unix file system as their origin, so that the
# archive does not depend on the platform that assembled it.
ZIP_CREATOR_UNIX = 3

PKG_CONFIG = """prefix=/usr/local
libdir=${{prefix}}/lib
includedir=${{prefix}}/include

Name: nurt
Description: C interface of the Danaco Nurt library
Version: {version}
Cflags: -I${{includedir}}
Libs: -L${{libdir}} -lnurt
"""


class PackError(Exception):
    """A reason the archive cannot be assembled."""


def parse_arguments(argv):
    parser = argparse.ArgumentParser(description="Assemble the release archive of one target.")
    parser.add_argument("--root", required=True, type=Path, help="repository directory")
    parser.add_argument("--version", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--binary", required=True, type=Path, help="nurt or nurt.exe")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--library", type=Path, help="shared library; packed with the header into nurt-lib-<version>-<target>")
    parser.add_argument("--man", type=Path, help="directory with the manual pages")
    parser.add_argument("--completions", type=Path, help="directory with the completion scripts")
    parser.add_argument("--site", type=Path, help="directory of the documentation site")
    parser.add_argument("--epoch", type=int, default=None, help="time of the entries, seconds")
    return parser.parse_args(argv)


def library_name(target):
    """Return the file name of the shared library in the archive of a target."""
    if target.endswith("-unknown-linux-gnu"):
        return "libnurt.so.1"
    if target.endswith("-apple-darwin"):
        return "libnurt.1.dylib"
    if "windows" in target:
        return "nurt.dll"
    raise PackError(
        f"--library belongs to glibc Linux, macOS and Windows targets only, not {target}"
    )


def collect(arguments):
    """Return the entries as a mapping from path in the archive to file path or bytes."""
    windows = "windows" in arguments.target
    entries = {}
    name = "nurt.exe" if windows else "nurt"
    entries[name] = arguments.binary
    for text in ROOT_TEXTS:
        source = arguments.root / text
        if not source.is_file():
            raise PackError(f"{source} does not exist")
        entries[text] = source
    if arguments.man is not None and not windows:
        for page in sorted(arguments.man.glob("*.[0-9]")):
            entries[f"man/man{page.suffix[1:]}/{page.name}"] = page
    if arguments.completions is not None:
        for script in sorted(arguments.completions.iterdir()):
            if script.is_file():
                entries[f"completions/{script.name}"] = script
    if arguments.site is not None:
        for path in sorted(arguments.site.rglob("*")):
            if path.is_file():
                entries[f"doc/{path.relative_to(arguments.site).as_posix()}"] = path
    for path, source in entries.items():
        if isinstance(source, Path) and not source.is_file():
            raise PackError(f"{source} does not exist (needed for {path})")
    return entries


def collect_library(arguments):
    """Return the entries of the library archive: library, header, pkg-config file, texts."""
    library = library_name(arguments.target)
    header = arguments.root / HEADER
    if not header.is_file():
        raise PackError(f"{header} does not exist")
    entries = {library: arguments.library, "nurt.h": header}
    if "windows" not in arguments.target:
        entries["nurt.pc"] = PKG_CONFIG.format(version=arguments.version).encode("utf-8")
    for text in LIBRARY_TEXTS:
        source = arguments.root / text
        if not source.is_file():
            raise PackError(f"{source} does not exist")
        entries[text] = source
    if not arguments.library.is_file():
        raise PackError(f"{arguments.library} does not exist (needed for {library})")
    return entries


def is_executable(path):
    return path in ("nurt", "nurt.exe")


def content(source):
    return source if isinstance(source, bytes) else source.read_bytes()


def write_tar(destination, top, entries, epoch):
    buffer = io.BytesIO()
    with gzip.GzipFile(fileobj=buffer, mode="wb", mtime=0, compresslevel=9) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
            directories = {top}
            for path in entries:
                parts = path.split("/")
                for depth in range(1, len(parts)):
                    directories.add(f"{top}/{'/'.join(parts[:depth])}")
            for directory in sorted(directories):
                info = tarfile.TarInfo(directory)
                info.type = tarfile.DIRTYPE
                info.mode, info.mtime = 0o755, epoch
                info.uid = info.gid = 0
                info.uname = info.gname = "root"
                archive.addfile(info)
            for path in sorted(entries):
                data = content(entries[path])
                info = tarfile.TarInfo(f"{top}/{path}")
                info.size = len(data)
                info.mode = 0o755 if is_executable(path) else 0o644
                info.mtime = epoch
                info.uid = info.gid = 0
                info.uname = info.gname = "root"
                archive.addfile(info, io.BytesIO(data))
    destination.write_bytes(buffer.getvalue())


def write_zip(destination, entries, epoch):
    stamp = time.gmtime(max(epoch, 315532800))[:6]
    with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in sorted(entries):
            info = zipfile.ZipInfo(path, date_time=stamp)
            info.create_system = ZIP_CREATOR_UNIX
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (0o755 if is_executable(path) else 0o644) << 16
            archive.writestr(info, content(entries[path]))


def main(argv):
    arguments = parse_arguments(argv)
    if not SEMVER.match(arguments.version):
        raise PackError(f"invalid version: {arguments.version}")
    if not TARGET.match(arguments.target):
        raise PackError(f"invalid target: {arguments.target}")
    epoch = arguments.epoch
    if epoch is None:
        text = os.environ.get("SOURCE_DATE_EPOCH", "0")
        try:
            epoch = int(text)
        except ValueError:
            raise PackError(f"SOURCE_DATE_EPOCH is not a number of seconds: {text!r}") from None
        if epoch < 0:
            raise PackError(f"SOURCE_DATE_EPOCH is negative: {epoch}")
    archives = [(f"nurt-{arguments.version}-{arguments.target}", collect(arguments))]
    if arguments.library is not None:
        archives.append((f"nurt-lib-{arguments.version}-{arguments.target}", collect_library(arguments)))
    arguments.output.mkdir(parents=True, exist_ok=True)
    for stem, entries in archives:
        if "windows" in arguments.target:
            destination = arguments.output / f"{stem}.zip"
            write_zip(destination, entries, epoch)
        else:
            destination = arguments.output / f"{stem}.tar.gz"
            write_tar(destination, stem, entries, epoch)
        print(f"{destination} written ({len(entries)} files)")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except PackError as error:
        print(f"pack.py: {error}", file=sys.stderr)
        sys.exit(1)
