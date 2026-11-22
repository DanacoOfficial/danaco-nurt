# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Building blocks of the parity scenarios: snapshots, typed record values and cases.

A parity scenario states a file system before, a program, the expected report
fields and the file system after. The helpers keep the scenario definitions
short and produce the JSON form of the case format.
"""

from __future__ import annotations

import base64
import json
from dataclasses import dataclass, field
from pathlib import Path

import suite

UNCHANGED = "unchanged"

FIELD_TYPES = {
    "path": "path",
    "name": "text",
    "stem": "text",
    "extension": "text",
    "parent": "path",
    "type": "enum",
    "exists": "bool",
    "size": "size",
    "text": "text",
    "line": "text",
    "lines": "int",
    "total_lines": "int",
    "no": "int",
    "column": "int",
    "count": "int",
    "match_count": "int",
    "encoding": "text",
    "line_ending": "enum",
    "bom": "bool",
    "binary": "bool",
    "unchanged": "bool",
    "hash": "text",
    "depth": "int",
    "relative": "text",
    "before": "list",
    "after": "list",
    "groups": "list",
    "key": "key",
    "value": "any",
    "hidden": "bool",
    "mode": "text",
    "link_target": "path",
    "cell_type": "enum",
    "index": "int",
    "source": "text",
    "truncated": "bool",
    "total_size": "size",
    "file_count": "int",
    "kind": "text",
    "state": "enum",
}
"""Value type of every record field used by the scenarios."""


def tagged(name: str, value: object) -> dict:
    """Returns a record field value in its tagged JSON form."""
    kind = FIELD_TYPES[name]
    if kind == "list":
        return {"t": "list", "v": [{"t": "text", "v": v} for v in value]}
    if kind == "any":
        if isinstance(value, bool):
            return {"t": "bool", "v": value}
        if isinstance(value, int):
            return {"t": "int", "v": value}
        return {"t": "text", "v": value}
    if kind == "path" and isinstance(value, str) and not value.startswith(("${base}", "/")):
        value = "${base}/" + value
    return {"t": kind, "v": value}


def row(record_kind: str, /, **fields: object) -> dict:
    """Returns an expected record with some of its fields."""
    return {"kind": record_kind, "fields": {k: tagged(k, v) for k, v in fields.items()}}


def records(kind: str, rows: list[dict] | None = None, matched: int | None = None) -> dict:
    """Returns the expected member records of a reading program."""
    out: dict = {"type": kind}
    if rows is not None:
        out["rows"] = rows
        out["matched"] = len(rows) if matched is None else matched
    elif matched is not None:
        out["matched"] = matched
    return out


def entry(path: str, content: str | bytes | None = None, *, kind: str | None = None,
          mode: str | None = None, target: str | None = None) -> dict:
    """Returns a snapshot entry: a file with text or bytes, a directory or a link."""
    if target is not None:
        return {"path": path, "type": "symlink", "target": target}
    if content is None and kind in (None, "dir"):
        return {"path": path, "type": "dir"}
    out: dict = {"path": path, "type": "file"}
    if isinstance(content, bytes):
        out["content_base64"] = base64.b64encode(content).decode("ascii")
    else:
        out["content_utf8"] = content
    if mode is not None:
        out["mode"] = mode
    return out


def tree(spec: dict[str, str | bytes | None]) -> dict:
    """Returns a snapshot from a mapping of paths to contents; None is a directory."""
    return {"entries": [entry(p, c) for p, c in spec.items()]}


def changed(before: dict[str, str | bytes | None], **updates: str | bytes | None) -> dict:
    """Returns a snapshot equal to `before` with files replaced, added or removed.

    Keyword names use `__` for `/` and `_dot_` for `.`; a value of `REMOVED`
    removes the entry.
    """
    state = dict(before)
    for key, value in updates.items():
        path = key.replace("__", "/").replace("_dot_", ".")
        if value is REMOVED:
            state.pop(path, None)
        else:
            state[path] = value
    return tree(state)


class _Removed:
    """Marker of a removed entry."""


REMOVED = _Removed()


@dataclass
class Collection:
    """Cases of one collection file: parity scenarios by tool and topic, or journal cases by topic."""

    tool: str
    topic: str | None
    kind: str = "parity"
    cases: list[dict] = field(default_factory=list)

    @property
    def path(self) -> Path:
        """File of the collection."""
        if self.topic is None:
            return suite.CONFORMANCE / self.kind / f"{self.tool}.cases.json"
        return suite.CONFORMANCE / self.kind / self.tool / f"{self.topic}.cases.json"

    def add(self, name: str, summary: str, before: dict | None, program: str | None = None, *,
            report: dict | None = None, after: object = UNCHANGED, options: dict | None = None,
            steps: list[dict] | None = None, feature: str | None = None,
            requires: tuple[str, ...] = (), config: str | None = None) -> None:
        """Adds one case."""
        parts = [self.kind, self.tool] + ([self.topic] if self.topic else []) + [name]
        case: dict = {"id": "/".join(parts), "kind": self.kind}
        if config is not None:
            case["config"] = config
        if feature is not None:
            case["class"] = f"optional:{feature}"
        if requires:
            case["requires"] = list(requires)
        case["summary"] = summary
        if before is not None:
            case["fs_before"] = before
        if steps is not None:
            case["step"] = steps
        else:
            case["program"] = program
            if options:
                case["options"] = options
            if report is not None:
                case["expect"] = {"report": report}
            case["fs_after"] = after
        self.cases.append(case)

    def render(self) -> str:
        """Returns the JSON text of the collection."""
        doc = {"format": suite.CASES_FORMAT, "case": self.cases}
        return json.dumps(doc, ensure_ascii=False, indent=2) + "\n"


def step(program: str | None = None, *, report: dict | None = None, after: object = None,
         options: dict | None = None, params: dict | None = None) -> dict:
    """Returns one step of a scenario."""
    out: dict = {}
    if params is not None:
        out["params"] = params
    if program is not None:
        out["program"] = program
    if options:
        out["options"] = options
    if report is not None:
        out["expect"] = {"report": report}
    if after is not None:
        out["fs_after"] = after
    return out


