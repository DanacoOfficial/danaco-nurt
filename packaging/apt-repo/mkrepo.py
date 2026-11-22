#!/usr/bin/env python3
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Generate an apt repository from a directory of Debian packages.

The output directory receives the layout

  pool/<component>/<initial>/<source>/<package file>
  dists/<suite>/<component>/binary-<arch>/Packages{,.gz,.xz}
  dists/<suite>/Release
  dists/<suite>/InRelease and dists/<suite>/Release.gpg   (signed only)

Signing uses the gpg program and a key named by --sign-key (a key identifier
or fingerprint known to the local keyring). The key is supplied when the
release is made; neither a private key nor an identifier is stored in the
repository. Without --sign-key the repository is left unsigned.

Usage:

  mkrepo.py --packages DIR --output DIR [--suite stable] [--component main]
            [--sign-key KEY] [--gpg-home DIR]

SOURCE_DATE_EPOCH, when set, fixes the Date field of the Release file.
"""

import argparse
import datetime
import gzip
import hashlib
import lzma
import os
import shutil
import subprocess
import sys
from email.utils import format_datetime
from pathlib import Path

ORIGIN = "Danaco"
LABEL = "Danaco Nurt"
DESCRIPTION = "Packages of Danaco Nurt"
HASHES = (("SHA256", hashlib.sha256),)
COMPRESSIONS = (("", None), (".gz", "gz"), (".xz", "xz"))


class RepoError(Exception):
    """The package set or the signing step is not usable."""


def read_control(package):
    """Return the control fields of a package as an ordered list of pairs."""
    result = subprocess.run(
        ["dpkg-deb", "--field", str(package)],
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    if result.returncode != 0:
        raise RepoError(f"{package.name}: {result.stderr.strip()}")
    fields = []
    for line in result.stdout.splitlines():
        if line[:1] in (" ", "\t") and fields:
            fields[-1] = (fields[-1][0], fields[-1][1] + "\n" + line)
        elif ":" in line:
            name, _, value = line.partition(":")
            fields.append((name, value.strip()))
    return fields


def file_digests(path):
    data = path.read_bytes()
    return len(data), {name: factory(data).hexdigest() for name, factory in HASHES}


def pool_directory(component, source_name):
    initial = source_name[:4] if source_name.startswith("lib") else source_name[:1]
    return Path("pool") / component / initial / source_name


def stanza(fields, filename, size, digests):
    lines = []
    for name, value in fields:
        lines.append(f"{name}: {value}")
    lines.append(f"Filename: {filename}")
    lines.append(f"Size: {size}")
    for name, digest in digests.items():
        lines.append(f"{name}: {digest}")
    return "\n".join(lines) + "\n"


def collect(packages_dir, output, component):
    """Copy the packages into the pool; return {arch: [stanza text]}."""
    packages = sorted(packages_dir.glob("*.deb"))
    if not packages:
        raise RepoError(f"no .deb files in {packages_dir}")
    by_arch = {}
    for package in packages:
        fields = dict(read_control(package))
        for required in ("Package", "Version", "Architecture"):
            if required not in fields:
                raise RepoError(f"{package.name}: control field {required} missing")
        name = fields["Package"]
        source = fields.get("Source", name).split()[0]
        relative = pool_directory(component, source) / package.name
        target = output / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(package, target)
        size, digests = file_digests(target)
        text = stanza(read_control(package), relative.as_posix(), size, digests)
        by_arch.setdefault(fields["Architecture"], []).append(text)
    return by_arch


def write_indices(by_arch, suite_dir, component):
    """Write Packages indices; return the paths relative to suite_dir."""
    declared = sorted(arch for arch in by_arch if arch != "all")
    if not declared:
        raise RepoError("no architecture-specific packages")
    written = []
    for arch in declared:
        stanzas = list(by_arch[arch]) + list(by_arch.get("all", []))
        text = "\n".join(stanzas).encode("utf-8")
        directory = suite_dir / component / f"binary-{arch}"
        directory.mkdir(parents=True, exist_ok=True)
        for suffix, kind in COMPRESSIONS:
            path = directory / f"Packages{suffix}"
            if kind == "gz":
                with open(path, "wb") as raw:
                    with gzip.GzipFile(fileobj=raw, mode="wb", mtime=0, compresslevel=9) as out:
                        out.write(text)
            elif kind == "xz":
                path.write_bytes(lzma.compress(text, preset=9))
            else:
                path.write_bytes(text)
            written.append(path.relative_to(suite_dir))
    return declared, written


def release_text(suite, component, architectures, files, suite_dir, epoch):
    moment = datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc)
    lines = [
        f"Origin: {ORIGIN}",
        f"Label: {LABEL}",
        f"Suite: {suite}",
        f"Codename: {suite}",
        f"Date: {format_datetime(moment, usegmt=True)}",
        f"Architectures: {' '.join(architectures)}",
        f"Components: {component}",
        f"Description: {DESCRIPTION}",
    ]
    for hash_name, factory in HASHES:
        lines.append(f"{hash_name}:")
        for relative in sorted(files):
            data = (suite_dir / relative).read_bytes()
            lines.append(f" {factory(data).hexdigest()} {len(data):>16} {relative.as_posix()}")
    return "\n".join(lines) + "\n"


def sign(suite_dir, key, gpg_home):
    """Create InRelease and Release.gpg for the Release file."""
    if shutil.which("gpg") is None:
        raise RepoError("gpg is not installed")
    base = ["gpg", "--batch", "--yes", "--local-user", key]
    if gpg_home:
        base[1:1] = ["--homedir", str(gpg_home)]
    release = suite_dir / "Release"
    commands = (
        base + ["--clearsign", "--output", str(suite_dir / "InRelease"), str(release)],
        base + ["--armor", "--detach-sign", "--output", str(suite_dir / "Release.gpg"), str(release)],
    )
    for command in commands:
        result = subprocess.run(command, check=False, capture_output=True, text=True)
        if result.returncode != 0:
            raise RepoError("signing failed: " + result.stderr.strip())


def build_parser():
    parser = argparse.ArgumentParser(description="Generate an apt repository.")
    parser.add_argument("--packages", required=True, type=Path, help="directory with .deb files")
    parser.add_argument("--output", required=True, type=Path, help="repository root to write")
    parser.add_argument("--suite", default="stable", help="suite and codename (default: stable)")
    parser.add_argument("--component", default="main", help="component (default: main)")
    parser.add_argument("--sign-key", help="key identifier for InRelease and Release.gpg")
    parser.add_argument("--gpg-home", type=Path, help="gpg home directory holding the key")
    return parser


def main(argv=None):
    args = build_parser().parse_args(argv)
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH") or datetime.datetime.now().timestamp())
    try:
        by_arch = collect(args.packages, args.output, args.component)
        suite_dir = args.output / "dists" / args.suite
        architectures, indices = write_indices(by_arch, suite_dir, args.component)
        release = release_text(args.suite, args.component, architectures, indices, suite_dir, epoch)
        (suite_dir / "Release").write_text(release, encoding="utf-8", newline="\n")
        for stale in ("InRelease", "Release.gpg"):
            (suite_dir / stale).unlink(missing_ok=True)
        if args.sign_key:
            sign(suite_dir, args.sign_key, args.gpg_home)
    except (RepoError, OSError) as error:
        print(f"mkrepo.py: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
