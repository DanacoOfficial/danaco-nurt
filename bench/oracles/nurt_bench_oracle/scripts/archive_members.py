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

import io
import lzma
import tarfile
import zipfile
import zlib
from typing import Any, Mapping

from ..snapshot import Index

MAX_UNPACKED_BYTES = 256 * 1024 * 1024
"""Largest amount of content read from one archive; a larger archive is not checked."""


def bounded(data: bytes, kind: str) -> bytes:
    """Decompresses the members of a gzip or xz stream, refusing more than
    ``MAX_UNPACKED_BYTES`` of content."""
    pieces: list[bytes] = []
    total = 0
    while data:
        if kind == "xz":
            unpacker = lzma.LZMADecompressor()
        else:
            unpacker = zlib.decompressobj(wbits=zlib.MAX_WBITS | 16)
        text = unpacker.decompress(data, MAX_UNPACKED_BYTES + 1 - total)
        total += len(text)
        if total > MAX_UNPACKED_BYTES:
            raise ValueError("the archive holds more than the permitted amount of content")
        if not unpacker.eof:
            raise EOFError(f"the {kind} stream is incomplete")
        pieces.append(text)
        data = unpacker.unused_data.lstrip(b"\0")
    return b"".join(pieces)


def read_members(data: bytes, kind: str) -> dict[str, bytes]:
    """Returns the regular files of an archive as a name to content mapping."""
    if kind == "zip":
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            if sum(info.file_size for info in archive.infolist()) > MAX_UNPACKED_BYTES:
                raise ValueError("the archive holds more than the permitted amount of content")
            return {
                info.filename: archive.read(info)
                for info in archive.infolist()
                if not info.is_dir()
            }
    if kind == "gz":
        return {"": bounded(data, "gz")}
    if kind == "tar.xz":
        data = bounded(data, "xz")
    elif kind == "tar.gz":
        data = bounded(data, "gz")
    members: dict[str, bytes] = {}
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:") as archive:
        for info in archive.getmembers():
            if info.isfile():
                handle = archive.extractfile(info)
                members[info.name] = handle.read() if handle else b""
    return members


def normalize_name(name: str, prefix: str) -> str:
    """Removes ``./`` and the configured prefix from a member name.

    Backslashes are read as separators: some Windows archivers store member
    names with ``\\``.
    """
    name = name.replace("\\", "/")
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
    except (
        OSError,
        EOFError,
        ValueError,
        RuntimeError,
        NotImplementedError,
        zlib.error,
        lzma.LZMAError,
        tarfile.TarError,
        zipfile.BadZipFile,
    ) as error:
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
