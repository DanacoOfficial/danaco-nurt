# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Checks the members of an archive in the final state.

Parameters (``oracle.params``):

- ``path``: archive path relative to the task root;
- ``format``: ``tar``, ``tar.gz``, ``tar.xz``, ``zip`` or ``gz``;
- ``members``: mapping of member path to expected UTF-8 text, or ``null``
  when only the presence of the member is checked;
- ``exact``: when ``true`` (default), the archive contains no other files;
- ``prefix``: optional directory prefix removed from member names.

For ``gz`` the archive holds one stream; ``members`` has one key, which is
not compared.
"""

from __future__ import annotations

import gzip
import io
import lzma
import tarfile
import zipfile
from typing import Any, Mapping

from ..snapshot import Index


def read_members(data: bytes, kind: str) -> dict[str, bytes]:
    """Returns the regular files of an archive as a name to content mapping."""
    if kind == "zip":
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            return {
                info.filename: archive.read(info)
                for info in archive.infolist()
                if not info.is_dir()
            }
    if kind == "gz":
        return {"": gzip.decompress(data)}
    if kind == "tar.xz":
        data = lzma.decompress(data)
    elif kind == "tar.gz":
        data = gzip.decompress(data)
    members: dict[str, bytes] = {}
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:") as archive:
        for info in archive.getmembers():
            if info.isfile():
                handle = archive.extractfile(info)
                members[info.name] = handle.read() if handle else b""
    return members


def normalize_name(name: str, prefix: str) -> str:
    """Removes ``./`` and the configured prefix from a member name."""
    while name.startswith("./"):
        name = name[2:]
    if prefix and name.startswith(prefix):
        name = name[len(prefix) :]
    return name.lstrip("/")


def check(
    before: Index, after: Index, task: Mapping[str, Any], observation: Mapping[str, Any]
) -> dict[str, Any]:
    """Evaluates the archive named by the task parameters."""
    params = task["oracle"]["params"]
    path = params["path"]
    entry = after.get(path)
    if entry is None or entry.kind != "file":
        return {"completed": False, "failures": [f"{path}: archive missing"]}
    kind = params["format"]
    try:
        raw = read_members(entry.content or b"", kind)
    except (OSError, EOFError, lzma.LZMAError, tarfile.TarError, zipfile.BadZipFile) as error:
        return {"completed": False, "failures": [f"{path}: not a readable {kind} archive: {error}"]}
    expected: Mapping[str, str | None] = params["members"]
    if kind == "gz":
        text = next(iter(expected.values()))
        if text is not None and raw[""] != text.encode("utf-8"):
            return {"completed": False, "failures": [f"{path}: decompressed content differs"]}
        return {"completed": True, "failures": []}
    members = {normalize_name(name, params.get("prefix", "")): data for name, data in raw.items()}
    failures: list[str] = []
    for name, text in expected.items():
        if name not in members:
            failures.append(f"{path}: member {name} missing")
        elif text is not None and members[name] != text.encode("utf-8"):
            failures.append(f"{path}: member {name} has different content")
    if params.get("exact", True):
        for name in sorted(set(members) - set(expected)):
            failures.append(f"{path}: unexpected member {name}")
    return {"completed": not failures, "failures": failures}
