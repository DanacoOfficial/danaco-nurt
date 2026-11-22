# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Encoding and line ending classification of file content."""

from __future__ import annotations

import codecs

ENCODING_UTF8 = "utf-8"
ENCODING_UTF8_BOM = "utf-8-bom"
ENCODING_UTF16LE = "utf-16le"
ENCODING_UTF16BE = "utf-16be"
ENCODING_8BIT = "8bit"

LINE_LF = "lf"
LINE_CRLF = "crlf"
LINE_MIXED = "mixed"
LINE_NONE = "none"

BOMS = (
    (codecs.BOM_UTF8, ENCODING_UTF8_BOM, "utf-8-sig"),
    (codecs.BOM_UTF16_LE, ENCODING_UTF16LE, "utf-16"),
    (codecs.BOM_UTF16_BE, ENCODING_UTF16BE, "utf-16"),
)


def encoding_of(data: bytes) -> str:
    """Classifies the encoding of content by its byte order mark and validity."""
    for bom, name, _ in BOMS:
        if data.startswith(bom):
            return name
    try:
        data.decode("utf-8")
    except UnicodeDecodeError:
        return ENCODING_8BIT
    return ENCODING_UTF8


def decode(data: bytes, codec: str | None = None) -> str:
    """Decodes content to text.

    Without ``codec``, a byte order mark selects UTF-8 or UTF-16 and other
    content is read as UTF-8 with replacement characters.
    """
    if codec:
        return data.decode(codec, errors="replace")
    for bom, _, python_codec in BOMS:
        if data.startswith(bom):
            return data.decode(python_codec, errors="replace")
    return data.decode("utf-8", errors="replace")


def line_endings_of(text: str) -> str:
    """Classifies the line ending style of decoded text."""
    crlf = text.count("\r\n")
    lf = text.count("\n") - crlf
    if crlf and lf:
        return LINE_MIXED
    if crlf:
        return LINE_CRLF
    if lf:
        return LINE_LF
    return LINE_NONE


def split_lines(text: str) -> list[str]:
    """Splits text into lines independent of the line ending style.

    A final line terminator does not produce an empty last line.
    """
    lines = text.replace("\r\n", "\n").split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    return lines


def encoding_changed(before: bytes, after: bytes) -> bool:
    """Returns whether the encoding class of content changed.

    Pure ASCII content is compatible with UTF-8 in both directions.
    """
    old = encoding_of(before)
    new = encoding_of(after)
    if old == new:
        return False
    if {old, new} == {ENCODING_UTF8, ENCODING_8BIT}:
        return not (is_ascii(before) or is_ascii(after))
    return True


def line_endings_changed(before: bytes, after: bytes) -> bool:
    """Returns whether a uniform line ending style was replaced by another."""
    old = line_endings_of(decode(before))
    new = line_endings_of(decode(after))
    if old not in (LINE_LF, LINE_CRLF) or new == LINE_NONE:
        return False
    return old != new


def is_ascii(data: bytes) -> bool:
    """Returns whether all bytes are 7-bit ASCII."""
    return all(byte < 0x80 for byte in data)
