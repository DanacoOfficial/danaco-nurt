# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Builders of snapshots and tasks shared by the tests."""

from __future__ import annotations

import base64
from pathlib import Path
from typing import Any

ORACLES_DIR = Path(__file__).resolve().parents[1]
SNAPSHOT_FORMAT = "nurt-fs-snapshot/1"


def entry(path: str, value: str | bytes | None, mode: str | None = None) -> dict[str, Any]:
    """Returns a snapshot entry; ``None`` stands for a directory."""
    if value is None:
        raw: dict[str, Any] = {"path": path, "type": "dir"}
    elif isinstance(value, bytes):
        raw = {"path": path, "type": "file", "content_base64": base64.b64encode(value).decode("ascii")}
    else:
        raw = {"path": path, "type": "file", "content_utf8": value}
    if mode is not None:
        raw["mode"] = mode
    return raw


def snap(files: dict[str, str | bytes | None], *extra: dict[str, Any]) -> dict[str, Any]:
    """Returns a snapshot of ``files`` followed by the ``extra`` entries."""
    entries = [entry(path, files[path]) for path in sorted(files)]
    return {"format": SNAPSHOT_FORMAT, "entries": entries + list(extra)}


def task(expect: dict[str, Any], forbidden: list[dict[str, str]] | None = None, **extra: Any) -> dict[str, Any]:
    """Returns the fields of a task that the oracle reads."""
    data: dict[str, Any] = {"id": "test-001", "expect": expect, "forbidden": forbidden or []}
    data.update(extra)
    return data
