#!/usr/bin/env python3
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Render the package manifest templates of a release.

Every file with the suffix ".in" below the source directory is a template.
The rendered file is written to the output directory under the same relative
path without the suffix. Placeholders are also expanded in relative paths.

Placeholders:

  @VERSION@          release version (SemVer 2.0.0)
  @DEB_VERSION@      Debian form: the first "-" of a pre-release becomes "~"
  @RPM_VERSION@      RPM Version field: the version without pre-release part
  @RPM_RELEASE@      RPM Release field: "1", or "0.<pre-release>" for a
                     pre-release
  @CHOCO_VERSION@    NuGet form: separators removed from the pre-release part
  @DATE@             release date, YYYY-MM-DD, from SOURCE_DATE_EPOCH
  @RPM_DATE@         release date in the form of RPM changelog entries
  @SHA256:<name>@    SHA-256 of the release file "nurt-<version>-<name>.tar.gz"
                     or "nurt-<version>-<name>.zip", read from checksums.txt

The checksums file has the format of "sha256sum": a hexadecimal digest, two
characters of separation and a file name per line.

Usage:

  render.py --checksums checksums.txt --output DIR [--source DIR]
            [--version VERSION] [--epoch SECONDS]
"""

import argparse
import datetime
import os
import re
import sys
from pathlib import Path

TEMPLATE_SUFFIX = ".in"
PRODUCT = "nurt"
ARCHIVE_SUFFIXES = (".tar.gz", ".zip")

SEMVER = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$"
)
DIGEST_LINE = re.compile(r"^([0-9a-fA-F]{64}) [ *](\S.*)$")
PLACEHOLDER = re.compile(r"@([A-Z][A-Z0-9_]*)(?::([A-Za-z0-9_.-]+))?@")


class RenderError(Exception):
    """A release input is missing or malformed."""


def parse_checksums(text):
    """Return a mapping from file name to lower-case SHA-256 digest."""
    digests = {}
    for number, line in enumerate(text.splitlines(), start=1):
        if not line.strip():
            continue
        match = DIGEST_LINE.match(line)
        if match is None:
            raise RenderError(f"checksums line {number} is not a sha256sum entry")
        name = os.path.basename(match.group(2))
        if name in digests:
            raise RenderError(f"checksums list {name} more than once")
        digests[name] = match.group(1).lower()
    return digests


def version_from_checksums(digests):
    """Return the single release version named by the archive file names."""
    pattern = re.compile(
        r"^" + PRODUCT + r"-(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+?)?)-"
        r"(?:(?:x86_64|aarch64)-[A-Za-z0-9_.-]+|source)(?:\.tar\.gz|\.zip)$"
    )
    versions = set()
    for name in digests:
        match = pattern.match(name)
        if match:
            versions.add(match.group(1))
    if not versions:
        raise RenderError("checksums name no release archive")
    if len(versions) > 1:
        raise RenderError("checksums name more than one version: " + ", ".join(sorted(versions)))
    return versions.pop()


def parse_version(version):
    """Return (core, pre-release) of a SemVer string."""
    match = SEMVER.match(version)
    if match is None:
        raise RenderError(f"version {version!r} is not a SemVer 2.0.0 version")
    core = ".".join(match.group(1, 2, 3))
    return core, match.group(4)


def release_values(version, epoch):
    """Return the mapping of the plain placeholders for a release."""
    core, pre = parse_version(version)
    values = {
        "VERSION": version,
        "DEB_VERSION": core + ("~" + pre if pre else ""),
        "RPM_VERSION": core,
        "RPM_RELEASE": "0." + pre.replace("-", ".") if pre else "1",
        "CHOCO_VERSION": core + ("-" + re.sub(r"[.-]", "", pre) if pre else ""),
    }
    if epoch is not None:
        moment = datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc)
        values["DATE"] = moment.strftime("%Y-%m-%d")
        values["RPM_DATE"] = (
            f"{('Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun')[moment.weekday()]} "
            f"{('Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec')[moment.month - 1]} "
            f"{moment.day:02d} {moment.year}"
        )
    return values


def lookup_digest(digests, version, name):
    """Return the digest of the release archive identified by a target name."""
    found = [
        digests[candidate]
        for candidate in (f"{PRODUCT}-{version}-{name}{suffix}" for suffix in ARCHIVE_SUFFIXES)
        if candidate in digests
    ]
    if not found:
        raise RenderError(f"checksums have no archive for {name!r} of version {version}")
    return found[0]


def render_text(text, values, digests, version, origin):
    """Expand the placeholders of one template."""

    def substitute(match):
        key, argument = match.group(1), match.group(2)
        if key == "SHA256":
            if argument is None:
                raise RenderError(f"{origin}: SHA256 placeholder without a target name")
            return lookup_digest(digests, version, argument)
        if argument is not None or key not in values:
            hint = " (set SOURCE_DATE_EPOCH or --epoch)" if key in ("DATE", "RPM_DATE") else ""
            raise RenderError(f"{origin}: unknown or unavailable placeholder @{key}@{hint}")
        return values[key]

    return PLACEHOLDER.sub(substitute, text)


def render_tree(source, output, digests, version, epoch):
    """Render all templates below source into output; return written paths."""
    values = release_values(version, epoch)
    templates = sorted(
        path for path in source.rglob("*") if path.is_file() and path.name.endswith(TEMPLATE_SUFFIX)
    )
    if not templates:
        raise RenderError(f"no templates below {source}")
    written = []
    for template in templates:
        relative = template.relative_to(source).as_posix()
        target_relative = render_text(
            relative[: -len(TEMPLATE_SUFFIX)], values, digests, version, relative
        )
        target = output / target_relative
        target.parent.mkdir(parents=True, exist_ok=True)
        content = render_text(
            template.read_text(encoding="utf-8"), values, digests, version, relative
        )
        target.write_text(content, encoding="utf-8", newline="\n")
        target.chmod(template.stat().st_mode & 0o777)
        written.append(target)
    return written


def build_parser():
    parser = argparse.ArgumentParser(description="Render package manifests of a release.")
    parser.add_argument("--checksums", required=True, type=Path, help="checksums.txt of the release")
    parser.add_argument("--output", required=True, type=Path, help="directory for rendered files")
    parser.add_argument(
        "--source",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="directory searched for templates (default: the packaging directory)",
    )
    parser.add_argument("--version", help="release version (default: taken from checksums)")
    parser.add_argument(
        "--epoch",
        type=int,
        help="release time in seconds since the epoch (default: SOURCE_DATE_EPOCH)",
    )
    return parser


def main(argv=None):
    args = build_parser().parse_args(argv)
    try:
        digests = parse_checksums(args.checksums.read_text(encoding="utf-8"))
        version = args.version or version_from_checksums(digests)
        epoch = args.epoch
        if epoch is None and os.environ.get("SOURCE_DATE_EPOCH"):
            epoch = int(os.environ["SOURCE_DATE_EPOCH"])
        written = render_tree(args.source, args.output, digests, version, epoch)
    except (RenderError, OSError, ValueError) as error:
        print(f"render.py: {error}", file=sys.stderr)
        return 1
    for path in written:
        print(path.relative_to(args.output).as_posix())
    return 0


if __name__ == "__main__":
    sys.exit(main())
