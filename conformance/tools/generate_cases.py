# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Writes the generated collection files of the parity and journal parts of the suite.

The scenario definitions live in `parity_files.py`, `parity_safety.py` and
`journal_cases.py`; the files under `conformance/parity/` and
`conformance/journal/` are their output and are not edited by hand.

Run from the repository root:
    python3 conformance/tools/generate_cases.py           writes every file
    python3 conformance/tools/generate_cases.py --check   exits with 1 when a file differs or is extra
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import journal_cases
import parity_files
import parity_safety
import suite
from parity_model import Collection

GENERATED_ROOTS = (suite.CONFORMANCE / "parity", suite.CONFORMANCE / "journal")
TOOL_MARKER = "_danaco"
"""Member that local editing tools may add to JSON objects; it is not part of the content."""


def collections() -> list[Collection]:
    """Returns every generated collection."""
    return parity_files.collections() + parity_safety.collections() + journal_cases.collections()


def build() -> dict[Path, str]:
    """Returns the text of every generated file by path."""
    files: dict[Path, str] = {}
    for collection in collections():
        if collection.path in files:
            raise ValueError(f"two collections write {collection.path}")
        files[collection.path] = collection.render()
    return files


def content(value: object) -> object:
    """Returns a JSON value without the members added by local editing tools."""
    if isinstance(value, dict):
        return {k: content(v) for k, v in value.items() if k != TOOL_MARKER}
    if isinstance(value, list):
        return [content(v) for v in value]
    return value


def stale_files(files: dict[Path, str]) -> list[str]:
    """Returns the generated files that differ from the definitions, are missing or are extra."""
    stale = []
    for path, text in files.items():
        if not path.is_file() or content(json.loads(path.read_text(encoding="utf-8"))) != json.loads(text):
            stale.append(str(path.relative_to(suite.REPO)))
    for root in GENERATED_ROOTS:
        for path in sorted(root.rglob("*.cases.json")) if root.is_dir() else []:
            if path not in files:
                stale.append(str(path.relative_to(suite.REPO)))
    return stale


def main(argv: list[str]) -> int:
    """Writes or compares the generated files."""
    files = build()
    if "--check" in argv:
        stale = stale_files(files)
        if stale:
            print("generated files are out of date: " + ", ".join(stale), file=sys.stderr)
            return 1
        return 0
    for path, text in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
