# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Snapshots of file trees in the format ``nurt-fs-snapshot/1``.

A snapshot is a JSON object ``{"format": "nurt-fs-snapshot/1", "entries":
[...]}``. Inside this package a snapshot is handled as an index: a mapping
from the relative path to an :class:`Entry`.
"""

from __future__ import annotations

import base64
import datetime
import json
import os
import stat
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

SNAPSHOT_FORMAT = "nurt-fs-snapshot/1"
DEFAULT_FILE_MODE = "0644"
DEFAULT_DIR_MODE = "0755"
PERMISSION_BITS = 0o7777
MTIME_FORMAT = "%Y-%m-%dT%H:%M:%SZ"


class SnapshotError(ValueError):
    """A snapshot or a fixture reference is malformed."""


@dataclass(frozen=True)
class Entry:
    """One entry of a file tree."""

    kind: str
    content: bytes | None = None
    mode: str | None = None
    target: str | None = None
    mtime: str | None = None

    def same_as(self, other: "Entry", compare_mode: bool) -> bool:
        """Returns whether two entries are equal for the comparison of states."""
        if self.kind != other.kind:
            return False
        if self.kind == "file" and self.content != other.content:
            return False
        if self.kind == "symlink" and self.target != other.target:
            return False
        if compare_mode and self.kind != "symlink" and self.mode and other.mode:
            return self.mode == other.mode
        return True


Index = dict[str, Entry]


def normalize_mode(value: Any) -> str | None:
    """Returns a mode as a four-digit octal string, or ``None``."""
    if value is None:
        return None
    if isinstance(value, int) and not isinstance(value, bool):
        number = value
    elif isinstance(value, str):
        try:
            number = int(value, 8)
        except ValueError as error:
            raise SnapshotError(f"invalid mode {value!r}") from error
    else:
        raise SnapshotError(f"invalid mode {value!r}")
    return format(number & PERMISSION_BITS, "04o")


def check_relative(path: str) -> str:
    """Validates a relative snapshot path and returns it."""
    if not path or path.startswith("/") or "\\" in path or ":" in path:
        raise SnapshotError(f"path must be relative and use '/': {path!r}")
    parts = path.split("/")
    if any(part in ("", ".", "..") for part in parts):
        raise SnapshotError(f"path has an empty, '.' or '..' segment: {path!r}")
    return path


def entry_from_json(raw: Mapping[str, Any]) -> tuple[str, Entry]:
    """Converts one snapshot entry to a path and an :class:`Entry`."""
    path = check_relative(str(raw.get("path", "")))
    kind = raw.get("type")
    if kind == "file":
        if "content_utf8" in raw and "content_base64" in raw:
            raise SnapshotError(f"{path}: both content_utf8 and content_base64")
        if "content_base64" in raw:
            content = base64.b64decode(raw["content_base64"], validate=True)
        else:
            content = str(raw.get("content_utf8", "")).encode("utf-8")
        return path, Entry("file", content, normalize_mode(raw.get("mode")), mtime=raw.get("mtime"))
    if kind == "dir":
        return path, Entry("dir", mode=normalize_mode(raw.get("mode")))
    if kind == "symlink":
        target = raw.get("target")
        if not isinstance(target, str) or not target:
            raise SnapshotError(f"{path}: symlink without target")
        return path, Entry("symlink", target=target)
    raise SnapshotError(f"{path}: unknown entry type {kind!r}")


def entry_to_json(path: str, entry: Entry) -> dict[str, Any]:
    """Converts an :class:`Entry` to a snapshot entry."""
    raw: dict[str, Any] = {"path": path, "type": entry.kind}
    if entry.kind == "file":
        data = entry.content or b""
        try:
            raw["content_utf8"] = data.decode("utf-8")
        except UnicodeDecodeError:
            raw["content_base64"] = base64.b64encode(data).decode("ascii")
    if entry.kind == "symlink":
        raw["target"] = entry.target
    if entry.mode is not None and entry.kind != "symlink":
        raw["mode"] = entry.mode
    return raw


def index_from_entries(entries: list[Mapping[str, Any]]) -> Index:
    """Builds an index from a list of snapshot entries."""
    index: Index = {}
    for raw in entries:
        path, entry = entry_from_json(raw)
        index[path] = entry
    return index


def index_from_snapshot(snapshot: Mapping[str, Any]) -> Index:
    """Builds an index from a snapshot object.

    The ``format`` member is optional; when present and not ``null`` it must
    be ``nurt-fs-snapshot/1``. The member ``exhaustive`` is accepted and not
    used: a snapshot taken by a runner always covers the whole task root.
    """
    if snapshot.get("format") not in (None, SNAPSHOT_FORMAT):
        raise SnapshotError(f"snapshot format must be {SNAPSHOT_FORMAT}")
    entries = snapshot.get("entries")
    if not isinstance(entries, list):
        raise SnapshotError("snapshot has no entries array")
    return index_from_entries(entries)


def snapshot_from_index(index: Mapping[str, Entry]) -> dict[str, Any]:
    """Builds a snapshot object from an index."""
    return {
        "format": SNAPSHOT_FORMAT,
        "entries": [entry_to_json(path, index[path]) for path in sorted(index)],
    }


def as_index(state: Mapping[str, Any] | Index) -> Index:
    """Accepts a snapshot object or an index and returns an index."""
    if isinstance(state.get("entries"), list):
        return with_parents(index_from_snapshot(state))
    return dict(state)


def with_parents(index: Mapping[str, Entry]) -> Index:
    """Returns the index with the implicit parent directories added."""
    result: Index = dict(index)
    for path in list(index):
        parts = path.split("/")
        for depth in range(1, len(parts)):
            parent = "/".join(parts[:depth])
            result.setdefault(parent, Entry("dir", mode=DEFAULT_DIR_MODE))
    return result


def resolve_fs_before(fs_before: Mapping[str, Any], fixtures_dir: Path) -> Index:
    """Resolves the ``fs_before`` field of a task to an index.

    # Errors
    Raises :class:`SnapshotError` when the snapshot or the fixture is invalid
    or the fixture file does not exist.
    """
    if "fixture" in fs_before:
        name = str(fs_before["fixture"])
        path = fixtures_dir / f"{name}.json"
        if not path.is_file():
            raise SnapshotError(f"fixture {name!r} not found")
        with path.open(encoding="utf-8") as handle:
            index = index_from_snapshot(json.load(handle))
        index.update(index_from_entries(list(fs_before.get("overlay", []))))
        return with_parents(index)
    return with_parents(index_from_snapshot(fs_before))


def materialize(index: Mapping[str, Entry], root: Path) -> None:
    """Creates the entries of an index below ``root``.

    Directories are created first, then files and links. Modification times
    are applied to files; modes are applied on POSIX systems only.
    """
    full = with_parents(index)
    for path in sorted(p for p, e in full.items() if e.kind == "dir"):
        (root / path).mkdir(parents=True, exist_ok=True)
    for path in sorted(full):
        entry = full[path]
        target = root / path
        if entry.kind == "file":
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(entry.content or b"")
        elif entry.kind == "symlink":
            target.parent.mkdir(parents=True, exist_ok=True)
            os.symlink(entry.target or "", target)
    for path in sorted(full):
        entry = full[path]
        if entry.kind == "file" and entry.mtime:
            stamp = datetime.datetime.strptime(entry.mtime, MTIME_FORMAT).replace(tzinfo=datetime.timezone.utc)
            os.utime(root / path, (stamp.timestamp(), stamp.timestamp()))
    if os.name == "posix":
        for path in sorted(full, reverse=True):
            entry = full[path]
            if entry.kind == "symlink":
                continue
            default = DEFAULT_FILE_MODE if entry.kind == "file" else DEFAULT_DIR_MODE
            os.chmod(root / path, int(entry.mode or default, 8))


def capture(root: Path) -> Index:
    """Takes a snapshot of the tree below ``root`` without following links."""
    index: Index = {}
    record_mode = os.name == "posix"
    stack = [root]
    while stack:
        directory = stack.pop()
        with os.scandir(directory) as scan:
            for item in scan:
                path = Path(item.path)
                relative = path.relative_to(root).as_posix()
                info = item.stat(follow_symlinks=False)
                mode = format(stat.S_IMODE(info.st_mode), "04o") if record_mode else None
                if item.is_symlink():
                    index[relative] = Entry("symlink", target=os.readlink(path))
                elif item.is_dir(follow_symlinks=False):
                    index[relative] = Entry("dir", mode=mode)
                    stack.append(path)
                else:
                    index[relative] = Entry("file", path.read_bytes(), mode)
    return index
