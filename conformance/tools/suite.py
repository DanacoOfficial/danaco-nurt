# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           conformance/tools/suite.py
# Responsibility:   Script suite.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Data model of the conformance cases for execution, journal, correction and parity.

The module reads the command catalog, the registries of the specification
(error codes, unusual effects, irreversible reasons, journal coverage) and the
case files of the suite. It also models the name correction rules that the
generated correction corpus relies on: the edit distance, the thresholds and
the uniqueness condition.
"""

from __future__ import annotations

import json
import re
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CONFORMANCE = REPO / "conformance"
CASES_FORMAT = "nurt-conformance-cases/1"
SNAPSHOT_FORMAT = "nurt-fs-snapshot/1"
LANGUAGE = "1.0"

KINDS = ("exec", "journal", "correction", "parity")
METHODS = (
    "run", "check", "correct", "page", "parse", "history", "cancel", "testkit.clock.advance",
    "testkit.failpoint.set",
)
STATUSES = ("ok", "planned", "running", "partial", "rolled_back", "failed")
FEATURES = ("journal", "testkit")
REQUIREMENTS = re.compile(r"^(os:(linux|macos|windows|unix)|feature:[a-z0-9-]+|fixture:(http|second-volume))$")
CLASS = re.compile(r"^(mandatory|optional:[a-z0-9-]+)$")
CASE_ID = re.compile(r"^[a-z0-9._/-]+$")
RULE_ID = re.compile(r"^[LTVBP][0-9]{2}$")

SYNONYMS = {
    "delete": "remove",
    "del": "remove",
    "erase": "remove",
    "rm": "remove",
    "work": "jobs",
    "ls": "list",
    "dir": "list",
    "gci": "list",
    "cat": "read",
    "type": "read",
    "gc": "read",
    "grep": "search",
    "sls": "search",
    "findstr": "search",
    "head": "first",
    "take": "first",
    "limit": "first",
    "tail": "last",
    "uniq": "distinct",
    "len": "count",
    "length": "count",
    "where": "filter",
    "stat": "info",
    "ps": "processes",
    "gps": "processes",
    "df": "disks",
    "kill": "stop",
    "env": "environment",
    "printenv": "environment",
    "which": "program.find",
}
"""Synonyms of command names that the correction maps without a distance."""

HABITS = (
    "cd", "ls", "cat", "grep", "sed", "awk", "rm", "mv", "cp", "mkdir", "touch",
    "chmod", "chown", "ln", "find", "xargs", "head", "tail", "echo", "export",
    "sudo", "for", "while", "if",
)
"""First words of shell statements that are read as shell habits."""

JOURNAL_CHANGING = ("undo", "redo", "journal.purge", "journal.pin", "journal.limits")

INACTIVE_IRREVERSIBLE = {
    "network_volume": "network_volume",
    "removable_volume": "removable_volume",
    "below_reserve": "no_space_for_copy",
    "service_down": "journal_inactive",
    "memory_volume": "journal_inactive",
    "no_anchor": "journal_inactive",
    "disabled": "journal_inactive",
    "not_durable": "journal_inactive",
}
"""Irreversible reason reported for a change on a volume where the journal is inactive, by inactive reason."""

OS_CONDITIONS = {
    "linux": {"os:linux", "os:unix"},
    "macos": {"os:macos", "os:unix"},
    "windows": {"os:windows"},
    "unix": {"os:linux", "os:macos", "os:unix"},
}
"""Conditions of `requires` under which a case can run on each system named by the catalog."""


@dataclass(frozen=True)
class Param:
    """Parameter of a catalog command."""

    name: str
    type: str
    default: str | None
    position: int | None
    required: bool
    values: tuple[str, ...]


@dataclass(frozen=True)
class Command:
    """Command of the catalog with the facts the checks use."""

    name: str
    group: str
    category: str
    reversibility: str
    effect: str
    params: dict[str, Param]
    os: tuple[str, ...]

    @property
    def changing(self) -> bool:
        """Whether the command changes or runs something, which tightens the correction."""
        return self.category in ("change", "run") or self.name in JOURNAL_CHANGING


def load_catalog() -> dict[str, Command]:
    """Returns every command of the catalog by name."""
    commands: dict[str, Command] = {}
    for path in sorted((REPO / "catalog").glob("*.toml")):
        if path.name.startswith("_"):
            continue
        data = tomllib.loads(path.read_text(encoding="utf-8"))
        for raw in data["command"]:
            params = {}
            for p in raw.get("param", []):
                params[p["name"]] = Param(
                    name=p["name"],
                    type=p["type"],
                    default=p.get("default"),
                    position=p.get("position"),
                    required=bool(p.get("required", False)),
                    values=tuple(p.get("values", ())),
                )
            commands[raw["name"]] = Command(
                name=raw["name"],
                group=data["group"],
                category=raw["category"],
                reversibility=raw["reversibility"],
                effect=raw["effect"],
                params=params,
                os=tuple(raw.get("os", ())),
            )
    return commands


def _table_codes(text: str, heading: str) -> list[str]:
    """Returns the backticked names in the first column of the table under a heading."""
    lines = text.splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith("#") and heading in line)
    codes: list[str] = []
    for line in lines[start + 1:]:
        if line.startswith("#"):
            break
        if not line.startswith("| `"):
            continue
        first = line.split("|")[1]
        codes.extend(re.findall(r"`([a-z_]+|E_[A-Z_]+)`", first))
    return codes


def error_codes() -> set[str]:
    """Returns the error codes of the registry."""
    text = (REPO / "spec" / "errors.md").read_text(encoding="utf-8")
    return set(_table_codes(text, "Codes"))


def notable_codes() -> set[str]:
    """Returns the codes of unusual effects."""
    text = (REPO / "spec" / "report.md").read_text(encoding="utf-8")
    return set(_table_codes(text, "Unusual effects"))


def irreversible_reasons() -> set[str]:
    """Returns the reasons of irreversible effects."""
    text = (REPO / "spec" / "report.md").read_text(encoding="utf-8")
    return set(_table_codes(text, "Irreversible effects"))


def journal_coverage() -> list[dict]:
    """Returns the rows of the journal coverage table."""
    data = tomllib.loads((REPO / "tables" / "journal-coverage.toml").read_text(encoding="utf-8"))
    return data["row"]


def coverage_key(row: dict) -> str:
    """Returns the case name of a coverage row: the command and the action, if any."""
    action = row.get("action")
    name = row["command"] if action is None else f"{row['command']}-{action}"
    return name.replace("_", "-")


@dataclass
class Case:
    """One case read from a case file."""

    data: dict
    file: Path

    @property
    def id(self) -> str:
        """Identifier of the case."""
        return self.data.get("id", "")

    def steps(self) -> list[dict]:
        """Returns the steps of the case; a case without steps is one step."""
        if "step" in self.data:
            return list(self.data["step"])
        return [self.data]


def case_files(kind: str | None = None) -> list[Path]:
    """Returns the case files of the suite parts covered by this module."""
    roots = [CONFORMANCE / k for k in (KINDS if kind is None else (kind,))]
    files: list[Path] = []
    for root in roots:
        if root.is_dir():
            files.extend(sorted(root.rglob("*.cases.toml")))
            files.extend(sorted(root.rglob("*.cases.json")))
    return files


def load_case_file(path: Path) -> dict:
    """Parses a case file written in TOML or JSON."""
    text = path.read_text(encoding="utf-8")
    if path.name.endswith(".toml"):
        return tomllib.loads(text)
    return json.loads(text)


def load_cases(kind: str | None = None) -> list[Case]:
    """Returns every case of the given kind, or of all kinds."""
    cases: list[Case] = []
    for path in case_files(kind):
        data = load_case_file(path)
        for raw in data.get("case", []):
            cases.append(Case(raw, path))
    return cases


@dataclass
class Stage:
    """Stage of a program as seen by the scanner: the command and its named parameters."""

    command: str
    params: list[str] = field(default_factory=list)


_TOKEN = re.compile(
    r"""
    (?P<space>[ \t\r]+)
  | (?P<newline>\n)
  | (?P<raw>r(?P<hashes>\#*)")
  | (?P<string>")
  | (?P<pipe>\|>)
  | (?P<open>[(\[{])
  | (?P<close>[)\]}])
  | (?P<named>[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*:(?!:))
  | (?P<ident>[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)
  | (?P<regex>/)
  | (?P<other>.)
    """,
    re.VERBOSE,
)


def _skip_string(text: str, i: int) -> int:
    """Returns the index after a string that starts after its opening quote."""
    while i < len(text):
        if text[i] == "\\":
            i += 2
            continue
        if text[i] == '"':
            return i + 1
        i += 1
    raise ValueError("unterminated text literal")


def _skip_regex(text: str, i: int) -> int:
    """Returns the index after a regular expression that starts after its first slash."""
    while i < len(text):
        if text[i] == "\\":
            i += 2
            continue
        if text[i] == "/":
            i += 1
            while i < len(text) and text[i] in "imsx":
                i += 1
            return i
        if text[i] == "\n":
            break
        i += 1
    raise ValueError("unterminated regular expression")


def scan_program(text: str, commands: dict[str, Command]) -> list[Stage]:
    """Returns the stages of a program written in variant A.

    The scanner recognises literals, brackets, pipes, named arguments and
    nested pipelines in parentheses. It does not validate the grammar; it
    serves the consistency checks of the case files.
    """
    stages: list[Stage] = []
    contexts: list[Stage | None] = [None]
    depths: list[int] = [0]
    expect_head = True
    previous = ""
    pending_let = False
    i = 0
    while i < len(text):
        m = _TOKEN.match(text, i)
        if m is None:
            raise ValueError(f"unexpected character at {i}")
        kind = m.lastgroup if m.lastgroup != "hashes" else "raw"
        token = m.group(0)
        i = m.end()
        if kind == "space":
            continue
        if kind == "newline":
            if depths[-1] == 0 and len(contexts) == 1:
                rest = text[i:].lstrip(" \t\r\n")
                if not rest.startswith("|>"):
                    expect_head = True
                    contexts[-1] = None
            previous = "newline"
            continue
        if kind == "raw":
            hashes = m.group("hashes") or ""
            end = text.find('"' + hashes, i)
            if end < 0:
                raise ValueError("unterminated raw text literal")
            i = end + 1 + len(hashes)
            expect_head = False
            previous = "literal"
            continue
        if kind == "string":
            i = _skip_string(text, i)
            expect_head = False
            previous = "literal"
            continue
        if kind == "regex":
            if previous in ("ident", "literal", "close"):
                previous = "other"
                continue
            i = _skip_regex(text, i)
            expect_head = False
            previous = "literal"
            continue
        if kind == "pipe":
            expect_head = True
            previous = "pipe"
            continue
        if kind == "open":
            call = previous == "ident" and text[m.start() - 1:m.start()].isalnum()
            depths[-1] += 1
            if token == "(" and not call:
                nxt = _TOKEN.match(text, i)
                while nxt is not None and nxt.lastgroup == "space":
                    nxt = _TOKEN.match(text, nxt.end())
                if nxt is not None and nxt.lastgroup == "ident" and nxt.group(0) in commands:
                    contexts.append(None)
                    depths.append(0)
                    expect_head = True
                    previous = "open"
                    continue
            expect_head = False
            previous = "open"
            continue
        if kind == "close":
            if depths[-1] == 0 and len(contexts) > 1:
                contexts.pop()
                depths.pop()
                depths[-1] -= 1
            else:
                depths[-1] -= 1
            expect_head = False
            previous = "close"
            continue
        if kind == "ident":
            if expect_head and token == "let":
                pending_let = True
                expect_head = False
                previous = "let"
                continue
            if expect_head:
                stage = Stage(token)
                stages.append(stage)
                contexts[-1] = stage
                expect_head = False
                previous = "head"
                continue
            previous = "ident"
            continue
        if kind == "named":
            stage = contexts[-1]
            if depths[-1] == 0 and stage is not None:
                stage.params.append(token[:-1])
            expect_head = False
            previous = "named"
            continue
        if token == "=" and pending_let:
            pending_let = False
            expect_head = True
            previous = "assign"
            continue
        if token == "@":
            expect_head = False
        previous = "other"
    return stages


def osa(a: str, b: str) -> int:
    """Returns the optimal string alignment distance between two names."""
    rows = len(a) + 1
    cols = len(b) + 1
    d = [[0] * cols for _ in range(rows)]
    for i in range(rows):
        d[i][0] = i
    for j in range(cols):
        d[0][j] = j
    for i in range(1, rows):
        for j in range(1, cols):
            cost = 0 if a[i - 1] == b[j - 1] else 1
            d[i][j] = min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + cost)
            if i > 1 and j > 1 and a[i - 1] == b[j - 2] and a[i - 2] == b[j - 1]:
                d[i][j] = min(d[i][j], d[i - 2][j - 2] + 1)
    return d[-1][-1]


def threshold(name: str) -> int:
    """Returns the largest distance at which a name is corrected."""
    if len(name) <= 3:
        return 0
    if len(name) <= 7:
        return 1
    return 2


def snake(name: str) -> str:
    """Returns a name in the canonical convention: lower case with underscores."""
    name = name.replace("-", "_")
    name = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", name)
    return name.lower()


@dataclass(frozen=True)
class Resolution:
    """Outcome of the name correction for one written name."""

    outcome: str
    name: str | None = None
    distance: int = 0
    tied: tuple[str, ...] = ()


def resolve_name(
    written: str,
    names: list[str],
    competitors: list[str] = (),
    changing: dict[str, bool] | None = None,
    require_first_char: bool = True,
) -> Resolution:
    """Applies the fuzzy name rules to one written name.

    The outcome is `exact` (a known name in another letter case or convention),
    `fuzzy` (a unique candidate within the threshold), `ambiguous` (a unique
    candidate of a changing command at distance 2), `tie` (several candidates
    at the same distance) or `none`.
    """
    norm = snake(written)
    if norm in names:
        return Resolution("exact", norm)
    pool = sorted(set(names) | set(competitors))
    scored = sorted((osa(norm, v), v) for v in pool)
    best = scored[0][0]
    tied = tuple(v for d, v in scored if d == best)
    if len(tied) > 1:
        return Resolution("tie", None, best, tied)
    target = tied[0]
    if target not in names:
        return Resolution("none", None, best, tied)
    if best > threshold(target):
        return Resolution("none", None, best, tied)
    if require_first_char and norm[:1] != target[:1]:
        return Resolution("none", None, best, tied)
    if changing and changing.get(target) and best > 1:
        return Resolution("ambiguous", target, best, tied)
    return Resolution("fuzzy", target, best, tied)


_RESOLVED: dict[tuple[frozenset[tuple[str, bool]], str], Resolution] = {}


def resolve_command(written: str, commands: dict[str, Command]) -> Resolution:
    """Applies the command name rules, with synonyms and shell habits as competitors.

    The result is cached by the content the rules read, the names and whether
    each command changes something, and not by the identity of the mapping: an
    identity can be handed to another mapping once the first one is gone.
    """
    key = (frozenset((name, command.changing) for name, command in commands.items()), written)
    if key in _RESOLVED:
        return _RESOLVED[key]
    norm = snake(written)
    if norm in SYNONYMS:
        result = Resolution("synonym", SYNONYMS[norm])
    else:
        competitors = list(SYNONYMS) + list(HABITS)
        changing = {name: c.changing for name, c in commands.items()}
        result = resolve_name(written, list(commands), competitors, changing)
    _RESOLVED[key] = result
    return result


def snapshot_errors(snapshot: object) -> list[str]:
    """Returns the problems of an inline or file snapshot."""
    if snapshot == "unchanged":
        return []
    if not isinstance(snapshot, dict):
        return ["snapshot is not an object"]
    problems: list[str] = []
    if "format" in snapshot and snapshot["format"] != SNAPSHOT_FORMAT:
        problems.append(f"unknown snapshot format {snapshot['format']!r}")
    for entry in snapshot.get("entries", []):
        path = entry.get("path", "")
        parts = path.split("/")
        if not path or path.startswith("/") or "\\" in path or "" in parts or "." in parts or ".." in parts:
            problems.append(f"entry path {path!r} is not relative and normalised")
        kind = entry.get("type")
        content = [k for k in ("content_utf8", "content_base64") if k in entry]
        if kind == "file" and len(content) != 1:
            problems.append(f"file {path!r} needs exactly one content field")
        elif kind == "dir" and content:
            problems.append(f"directory {path!r} has content")
        elif kind == "symlink" and "target" not in entry:
            problems.append(f"link {path!r} has no target")
        elif kind not in ("file", "dir", "symlink"):
            problems.append(f"entry {path!r} has unknown type {kind!r}")
    return problems


def walk(value: object, key: str | None = None):
    """Yields every (key, value) pair of a nested structure."""
    if isinstance(value, dict):
        for k, v in value.items():
            yield k, v
            yield from walk(v, k)
    elif isinstance(value, list):
        for v in value:
            yield key, v
            yield from walk(v, key)

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
