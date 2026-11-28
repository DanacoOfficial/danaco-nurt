# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bench/oracles/nurt_bench_oracle/matchers.py
# Responsibility:   Script matchers.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Matchers of the ``expect`` field: files, the final answer and probes."""

from __future__ import annotations

import base64
import hashlib
import json
import re
import tomllib
from typing import Any, Mapping

from .snapshot import Index, normalize_mode
from .textform import decode, encoding_of, line_endings_of, split_lines

EXACT_MATCHERS = frozenset({"text", "base64", "sha256", "same_as_before"})
FORM_MATCHERS = frozenset({"encoding", "line_endings"})
FILE_MATCHERS = frozenset(
    {
        "absent",
        "type",
        "text",
        "base64",
        "sha256",
        "same_as_before",
        "contains",
        "not_contains",
        "regex",
        "lines",
        "lines_unordered",
        "json",
        "toml",
        "decode",
        "encoding",
        "line_endings",
        "mode",
        "target",
    }
)
TEXT_MATCHERS = frozenset(
    {"contains", "not_contains", "regex", "lines", "lines_unordered", "json", "toml"}
)
ANSWER_MATCHERS = frozenset({"contains", "not_contains", "regex", "case_sensitive"})


def file_failures(
    path: str,
    matcher: Mapping[str, Any],
    before: Index,
    after: Index,
    compare_mode: bool,
) -> list[str]:
    """Returns the unmet conditions of a file matcher; empty when it holds."""
    entry = after.get(path)
    if matcher.get("absent"):
        return [] if entry is None else [f"{path}: exists, expected absent"]
    if entry is None:
        return [f"{path}: missing"]
    expected_kind = matcher.get("type", "file" if has_content_matcher(matcher) else None)
    if expected_kind and entry.kind != expected_kind:
        return [f"{path}: is {entry.kind}, expected {expected_kind}"]
    failures: list[str] = []
    if "target" in matcher and entry.target != matcher["target"]:
        failures.append(f"{path}: link target {entry.target!r}, expected {matcher['target']!r}")
    if "mode" in matcher and compare_mode and entry.mode is not None:
        expected_mode = normalize_mode(matcher["mode"])
        if entry.mode != expected_mode:
            failures.append(f"{path}: mode {entry.mode}, expected {expected_mode}")
    if entry.kind == "file":
        failures.extend(content_failures(path, matcher, entry.content or b"", before))
    return failures


def has_content_matcher(matcher: Mapping[str, Any]) -> bool:
    """Returns whether a matcher tests file content."""
    return bool(set(matcher) & (EXACT_MATCHERS | TEXT_MATCHERS | FORM_MATCHERS))


def content_failures(
    path: str, matcher: Mapping[str, Any], data: bytes, before: Index
) -> list[str]:
    """Returns the unmet content conditions of a file matcher."""
    failures: list[str] = []
    if "text" in matcher and data != str(matcher["text"]).encode("utf-8"):
        failures.append(f"{path}: content differs from the expected text")
    if "base64" in matcher and data != base64.b64decode(matcher["base64"]):
        failures.append(f"{path}: content differs from the expected bytes")
    if "sha256" in matcher and hashlib.sha256(data).hexdigest() != matcher["sha256"].lower():
        failures.append(f"{path}: SHA-256 differs")
    if "same_as_before" in matcher:
        source = before.get(matcher["same_as_before"])
        if source is None or source.kind != "file":
            failures.append(f"{path}: source {matcher['same_as_before']} not in the initial state")
        elif source.content != data:
            failures.append(f"{path}: content differs from {matcher['same_as_before']}")
    if "encoding" in matcher and encoding_of(data) != matcher["encoding"]:
        failures.append(f"{path}: encoding {encoding_of(data)}, expected {matcher['encoding']}")
    if not set(matcher) & (TEXT_MATCHERS | {"line_endings"}):
        return failures
    text = decode(data, matcher.get("decode"))
    if "line_endings" in matcher and line_endings_of(text) != matcher["line_endings"]:
        failures.append(
            f"{path}: line endings {line_endings_of(text)}, expected {matcher['line_endings']}"
        )
    failures.extend(text_failures(path, matcher, text))
    return failures


def text_failures(path: str, matcher: Mapping[str, Any], text: str) -> list[str]:
    """Returns the unmet conditions on decoded text."""
    failures: list[str] = []
    for needle in matcher.get("contains", []):
        if needle not in text:
            failures.append(f"{path}: does not contain {needle!r}")
    for needle in matcher.get("not_contains", []):
        if needle in text:
            failures.append(f"{path}: contains {needle!r}")
    for pattern in matcher.get("regex", []):
        if re.search(pattern, text, re.MULTILINE) is None:
            failures.append(f"{path}: does not match /{pattern}/")
    if "lines" in matcher and split_lines(text) != list(matcher["lines"]):
        failures.append(f"{path}: lines differ from the expected lines")
    if "lines_unordered" in matcher and sorted(split_lines(text)) != sorted(
        matcher["lines_unordered"]
    ):
        failures.append(f"{path}: set of lines differs from the expected lines")
    if "json" in matcher:
        try:
            value = json.loads(text.lstrip("\ufeff"))
        except ValueError:
            failures.append(f"{path}: not valid JSON")
        else:
            if value != matcher["json"]:
                failures.append(f"{path}: JSON value differs from the expected value")
    if "toml" in matcher:
        try:
            value = tomllib.loads(text)
        except tomllib.TOMLDecodeError:
            failures.append(f"{path}: not valid TOML")
        else:
            if value != matcher["toml"]:
                failures.append(f"{path}: TOML value differs from the expected value")
    return failures


def answer_failures(matcher: Mapping[str, Any], answer: str | None) -> list[str]:
    """Returns the unmet conditions on the final answer."""
    if answer is None:
        return ["no final answer"]
    sensitive = matcher.get("case_sensitive", True)

    def fold(text: str) -> str:
        text = text.replace("\\", "/")
        return text if sensitive else text.lower()

    haystack = fold(answer)
    flags = re.MULTILINE if sensitive else re.MULTILINE | re.IGNORECASE
    failures: list[str] = []
    for needle in matcher.get("contains", []):
        if fold(needle) not in haystack:
            failures.append(f"answer does not contain {needle!r}")
    for needle in matcher.get("not_contains", []):
        if fold(needle) in haystack:
            failures.append(f"answer contains {needle!r}")
    for pattern in matcher.get("regex", []):
        if re.search(pattern, answer, flags) is None:
            failures.append(f"answer does not match /{pattern}/")
    return failures


def normalize_stdout(text: str) -> str:
    """Normalizes probe output: LF line endings, no trailing white space."""
    return text.replace("\r\n", "\n").rstrip()


def probe_failures(
    expected: Mapping[str, Mapping[str, Any]], results: Mapping[str, Any] | None
) -> list[str]:
    """Returns the unmet conditions on probe results."""
    failures: list[str] = []
    results = results or {}
    for probe_id, fields in expected.items():
        result = results.get(probe_id)
        if not isinstance(result, Mapping):
            failures.append(f"probe {probe_id}: no result")
            continue
        for name, value in fields.items():
            actual = result.get(name)
            if name == "stdout" and isinstance(actual, str):
                actual = normalize_stdout(actual)
                value = normalize_stdout(str(value))
            if actual != value:
                failures.append(f"probe {probe_id}: {name} is {actual!r}, expected {value!r}")
    return failures


def is_exact(matcher: Mapping[str, Any]) -> bool:
    """Returns whether a matcher fixes the content or its form completely."""
    return bool(set(matcher) & (EXACT_MATCHERS | FORM_MATCHERS)) or bool(matcher.get("absent"))

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
