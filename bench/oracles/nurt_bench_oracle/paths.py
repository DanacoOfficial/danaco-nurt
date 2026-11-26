# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Path patterns of task files.

``*`` matches within one path segment, ``**`` matches any number of
segments, and ``dir/**`` also matches ``dir`` itself.
"""

from __future__ import annotations

import re
from functools import lru_cache
from typing import Iterable

CACHE_SIZE = 4096


@lru_cache(maxsize=CACHE_SIZE)
def compile_pattern(pattern: str) -> re.Pattern[str]:
    """Translates a path pattern to a regular expression."""
    parts = pattern.split("/")
    regex = ""
    for position, part in enumerate(parts):
        last = position == len(parts) - 1
        if part == "**":
            if last:
                regex = regex[:-1] + "(?:/.*)?" if regex else ".*"
            else:
                regex += "(?:[^/]+/)*"
            continue
        regex += translate_segment(part)
        if not last:
            regex += "/"
    return re.compile(regex + r"\Z")


def translate_segment(segment: str) -> str:
    """Translates one segment with ``*``, ``?`` and ``[...]`` to a regex."""
    out = []
    index = 0
    while index < len(segment):
        char = segment[index]
        if char == "*":
            out.append("[^/]*")
        elif char == "?":
            out.append("[^/]")
        elif char == "[":
            end = segment.find("]", index + 1)
            if end == -1 or segment[index + 1 : end] in ("", "!"):
                out.append(re.escape(char))
            else:
                body = segment[index + 1 : end].replace("\\", "\\\\")
                if body.startswith("!"):
                    body = "^" + body[1:]
                out.append(f"[{body}]")
                index = end
        else:
            out.append(re.escape(char))
        index += 1
    return "".join(out)


def matches(path: str, pattern: str) -> bool:
    """Returns whether a relative path matches a path pattern."""
    return compile_pattern(pattern).match(path) is not None


def matches_any(path: str, patterns: Iterable[str]) -> bool:
    """Returns whether a relative path matches one of the patterns."""
    return any(matches(path, pattern) for pattern in patterns)


def ancestors(path: str) -> list[str]:
    """Returns the parent directories of a relative path, outermost first."""
    parts = path.split("/")
    return ["/".join(parts[:depth]) for depth in range(1, len(parts))]
