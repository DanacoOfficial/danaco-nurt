# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Generates the mutation and adversarial corpora of the correction suite.

Every generated case starts from a valid seed program of one catalog command
and changes one name: a typing error in a command, parameter or enumeration
value, another letter case or naming convention, or a name that the correction
must leave alone. The expected outcome is derived from the catalog and the
name rules (distance, thresholds, strict uniqueness, first character, stricter
rules for changing commands), so the corpus follows the catalog when it
changes.

Usage:
    python3 conformance/tools/generate_correction.py          write the files
    python3 conformance/tools/generate_correction.py --check  compare only
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import suite

OUT = suite.CONFORMANCE / "correction" / "generated"

SEEDS: dict[str, str] = {
    "list": '{cmd} "/home/user/app"',
    "find": '{cmd} "/home/user/app" glob: "*.go"',
    "info": '{cmd} "/home/user/app/a.txt"',
    "read": '{cmd} "/home/user/app/a.txt"',
    "lines": '{cmd} "/home/user/app/a.txt"',
    "search": '{cmd} "TODO" "/home/user/app"',
    "filter": 'find "/home/user/app" |> {cmd} size > 1kb',
    "select": 'find "/home/user/app" |> {cmd} name size',
    "sort": 'find "/home/user/app" |> {cmd} size',
    "first": 'find "/home/user/app" |> {cmd} 5',
    "last": 'find "/home/user/app" |> {cmd} 5',
    "skip": 'find "/home/user/app" |> {cmd} 5',
    "count": 'find "/home/user/app" |> {cmd}',
    "assert": 'find "/home/user/app" glob: "*.tmp" |> {cmd} count: 1..',
    "verify": 'replace "a" "b" in: ["/home/user/app/a.txt"] |> {cmd} (run ["make", "test"] cwd: "/home/user/app" |> assert exit_code: 0)',
    "write": '{cmd} "/home/user/app/a.txt" "x\\n" mode: create',
    "replace": '{cmd} "a" "b" in: ["/home/user/app/a.txt"]',
    "insert": '{cmd} "x" in: "/home/user/app/a.txt" line: 1',
    "edit": '{cmd} "/home/user/app/a.txt" changes: [{replace: "a", with: "b"}]',
    "copy": '{cmd} "/home/user/app/a.txt" to: "/home/user/app/b.txt"',
    "move": '{cmd} "/home/user/app/a.txt" to: "/home/user/app/b.txt"',
    "remove": '{cmd} "/home/user/app/a.txt"',
    "rename": '{cmd} path: "/home/user/app/a.txt" to: "b.txt"',
    "create": '{cmd} "/home/user/app/out" type: directory',
    "attributes": '{cmd} "/home/user/app/run.sh" mode: "0755"',
    "run": '{cmd} ["make", "test"] cwd: "/home/user/app"',
    "jobs": "{cmd}",
    "stop": 'jobs "j1" |> {cmd}',
    "retry": 'jobs "j1" |> {cmd}',
    "history": "{cmd} limit: 5",
    "undo": "{cmd} 3",
    "redo": "{cmd} 3",
    "help": '{cmd} "find"',
    "set": '{cmd} "/home/user/app/package.json" key: "scripts.test" value: "vitest"',
    "query": '{cmd} ".name" path: "/home/user/app/package.json"',
    "sql": '{cmd} "/home/user/app/app.db" "select 1"',
    "diff": '{cmd} "/home/user/app/a.txt" "/home/user/app/b.txt"',
    "patch": '{cmd} from: "/home/user/app/fix.diff" root: "/home/user/app"',
    "hash": '{cmd} "/home/user/app/a.txt"',
    "convert": '{cmd} "/home/user/app/a.txt" line_ending: lf',
    "evaluate": "{cmd} 1 + 2",
    "group": 'find "/home/user/app" |> {cmd} extension',
    "summarize": 'find "/home/user/app" |> {cmd} size',
    "distinct": 'find "/home/user/app" |> {cmd} extension',
    "describe": 'find "/home/user/app" |> {cmd}',
    "parse": 'lines "/home/user/app/a.log" |> {cmd} /(?P<level>[A-Z]+): (?P<text>.*)/',
    "split": 'lines "/home/user/app/a.csv" |> {cmd} ","',
    "code.symbols": '{cmd} "parse" path: ["/home/user/app/src"]',
    "code.outline": '{cmd} "/home/user/app/src/main.rs"',
    "code.definition": '{cmd} "/home/user/app/src/main.rs" line: 10 column: 5',
    "code.references": '{cmd} "/home/user/app/src/main.rs" line: 10 column: 5',
    "code.hover": '{cmd} "/home/user/app/src/main.rs" line: 10 column: 5',
    "code.diagnostics": '{cmd} "/home/user/app/src"',
    "code.rename": 'code.symbols "oldName" path: ["/home/user/app/src"] kind: function |> {cmd} "newName"',
    "code.languages": "{cmd}",
    "code.implementations": '{cmd} "/home/user/app/src/main.rs" line: 10 column: 5',
    "code.type_definition": '{cmd} "/home/user/app/src/main.rs" line: 10 column: 5',
    "code.call_hierarchy": '{cmd} "/home/user/app/src/main.rs" line: 10 column: 5',
    "code.actions": '{cmd} "/home/user/app/src/main.rs" line: 10',
    "code.profile": '{cmd} "/home/user/app"',
    "notebook.cells": '{cmd} "/home/user/app/a.ipynb"',
    "notebook.insert": '{cmd} "/home/user/app/a.ipynb" index: 1 cell_type: code source: "print(1)"',
    "notebook.replace": '{cmd} "/home/user/app/a.ipynb" index: 1 source: "print(2)"',
    "notebook.move": '{cmd} "/home/user/app/a.ipynb" from: 1 to: 2',
    "notebook.copy": '{cmd} "/home/user/app/a.ipynb" from: 1 to: 2',
    "notebook.normalize": '{cmd} "/home/user/app/a.ipynb"',
    "notebook.ops": '{cmd} "/home/user/app/a.ipynb" changes: [{op: "remove", index: 2}]',
    "notebook.remove": '{cmd} "/home/user/app/a.ipynb" index: 2',
    "notebook.clear_outputs": '{cmd} "/home/user/app/a.ipynb"',
    "notebook.export": '{cmd} "/home/user/app/a.ipynb" to: "/home/user/app/a.py" format: script',
    "git.status": '{cmd} "/home/user/app"',
    "git.diff": '{cmd} "/home/user/app"',
    "git.log": '{cmd} "/home/user/app"',
    "git.show": '{cmd} "/home/user/app" "HEAD"',
    "git.blame": '{cmd} "/home/user/app/a.txt"',
    "git.branch": '{cmd} "/home/user/app"',
    "git.stage": '{cmd} "/home/user/app" action: list',
    "git.checkpoint": '{cmd} "/home/user/app" action: list',
    "git.review": '{cmd} "/home/user/app"',
    "git.worktree": '{cmd} "/home/user/app" action: list',
    "git.commit": '{cmd} "/home/user/app" message: "fix: parser"',
    "git.tag": '{cmd} "/home/user/app"',
    "git.fetch": '{cmd} "/home/user/app"',
    "git.clone": '{cmd} "https://example.com/repo.git" "/home/user/repo"',
    "git.switch": '{cmd} "/home/user/app" branch: "main"',
    "git.restore": '{cmd} "/home/user/app" paths: ["/home/user/app/a.txt"]',
    "git.reset": '{cmd} "/home/user/app" revision: "HEAD~1"',
    "git.merge": '{cmd} "/home/user/app" branch: "feature"',
    "git.rebase": '{cmd} "/home/user/app" onto: "main"',
    "git.stash": '{cmd} "/home/user/app" action: list',
    "git.pull": '{cmd} "/home/user/app"',
    "git.push": '{cmd} "/home/user/app" branch: "main"',
    "archive.list": '{cmd} "/home/user/app/a.zip"',
    "archive.extract": '{cmd} "/home/user/app/a.zip" into: "/home/user/out"',
    "archive.create": 'find "/home/user/app" |> {cmd} "/home/user/a.zip"',
    "compress": '{cmd} "/home/user/app/a.log"',
    "decompress": '{cmd} "/home/user/app/a.log.gz"',
    "processes": "{cmd}",
    "ports": "{cmd}",
    "disks": "{cmd}",
    "host": "{cmd}",
    "environment": "{cmd}",
    "open_files": '{cmd} "/home/user/app/a.log"',
    "accounts": "{cmd}",
    "sleep": "{cmd} 1s",
    "suspend": 'jobs "j1" |> {cmd}',
    "resume": 'jobs "j1" |> {cmd}',
    "logs": "{cmd}",
    "services": "{cmd}",
    "start": 'services name: "nginx" |> {cmd}',
    "restart": 'services name: "nginx" |> {cmd}',
    "enable": 'services name: "nginx" |> {cmd}',
    "disable": 'services name: "nginx" |> {cmd}',
    "windows.cim": '{cmd} "Win32_OperatingSystem"',
    "package.list": "{cmd}",
    "package.search": '{cmd} "curl"',
    "package.install": '{cmd} ["curl"]',
    "package.remove": '{cmd} ["curl"]',
    "package.upgrade": "{cmd}",
    "http.request": '{cmd} "https://example.com/api"',
    "http.download": '{cmd} "https://example.com/a.tar.gz" "/home/user/a.tar.gz"',
    "network.ping": '{cmd} "example.com"',
    "network.resolve": '{cmd} "example.com"',
    "network.interfaces": "{cmd}",
    "schedule.create": '{cmd} "backup" every: 1h action: (run ["make", "backup"] cwd: "/home/user/app")',
    "schedule.list": "{cmd}",
    "schedule.remove": '{cmd} "s1"',
    "schedule.pause": '{cmd} "s1"',
    "schedule.resume": '{cmd} "s1"',
    "schedule.run": '{cmd} "s1"',
    "schedule.history": "{cmd}",
    "journal.purge": "{cmd} up_to: 3",
    "journal.space": "{cmd}",
    "journal.status": "{cmd}",
    "journal.pin": "{cmd} 3",
    "journal.limits": "{cmd}",
    "program.find": '{cmd} "git"',
    "program.search": '{cmd} "pdf"',
    "program.describe": '{cmd} "git"',
    "program.catalog": "{cmd}",
    "test.run": '{cmd} "/home/user/app"',
    "test.failed": "{cmd}",
    "test.show": "{cmd}",
    "test.list": "{cmd}",
    "analyze.check": '{cmd} "/home/user/app"',
    "analyze.show": "{cmd}",
    "analyze.format": '{cmd} "/home/user/app/src"',
    "analyze.tools": '{cmd} "/home/user/app"',
}
"""Valid seed program of every command; `{cmd}` marks the stage under test, which is last."""

SKIPPED_PARAMS = {"notebook.ops"}
"""Commands whose seeds are not extended with parameters."""

NEIGHBOURS = {
    "a": "sq", "b": "vn", "c": "xv", "d": "sf", "e": "wr", "f": "dg", "g": "fh",
    "h": "gj", "i": "uo", "j": "hk", "k": "jl", "l": "k", "m": "n", "n": "bm",
    "o": "ip", "p": "o", "q": "w", "r": "et", "s": "ad", "t": "ry", "u": "yi",
    "v": "cb", "w": "qe", "x": "zc", "y": "tu", "z": "x",
}
"""Keys next to each letter on a QWERTY keyboard, used for substitution errors."""


def toml_string(value: str) -> str:
    """Returns a TOML string literal for a value."""
    if "'" not in value and "\n" not in value and "\\" not in value:
        return f"'{value}'"
    return json.dumps(value, ensure_ascii=False)


def edits(name: str) -> list[str]:
    """Returns single typing errors of a name that keep its first character.

    The order is fixed: transpositions, deletions, doubled letters and
    substitutions by a neighbouring key, each from the start of the name.
    Separators (`.` and `_`) are never touched.
    """
    out: list[str] = []
    for i in range(1, len(name) - 1):
        a, b = name[i], name[i + 1]
        if a != b and a.isalpha() and b.isalpha():
            out.append(name[:i] + b + a + name[i + 2:])
    for i in range(1, len(name)):
        if name[i].isalpha():
            out.append(name[:i] + name[i + 1:])
    for i in range(1, len(name)):
        if name[i].isalpha():
            out.append(name[:i] + name[i] + name[i:])
    for i in range(1, len(name)):
        for c in NEIGHBOURS.get(name[i], ""):
            out.append(name[:i] + c + name[i + 1:])
    seen: set[str] = set()
    unique = []
    for e in out:
        if e not in seen and e != name:
            seen.add(e)
            unique.append(e)
    return unique


def two_edits(name: str) -> list[str]:
    """Returns names with two transpositions of letters that do not overlap."""
    spots = [
        i for i in range(1, len(name) - 1)
        if name[i] != name[i + 1] and name[i].isalpha() and name[i + 1].isalpha()
    ]
    out = []
    for a in spots:
        for b in spots:
            if b >= a + 2:
                chars = list(name)
                chars[a], chars[a + 1] = chars[a + 1], chars[a]
                chars[b], chars[b + 1] = chars[b + 1], chars[b]
                out.append("".join(chars))
    return out


def slug(text: str) -> str:
    """Returns an identifier fragment for a name."""
    return text.replace(".", "-").replace("_", "-").lower()


class Writer:
    """Collects the cases of one generated file."""

    def __init__(self, name: str, summary: str) -> None:
        self.name = name
        self.summary = summary
        self.lines: list[str] = []
        self.ids: set[str] = set()

    def add(
        self,
        case_id: str,
        summary: str,
        program: str,
        expect_program: str | None = None,
        corrections: list[dict] | None = None,
        error: str | None = None,
        candidate: str | None = None,
        requires: tuple[str, ...] = (),
    ) -> None:
        """Adds one case; identifiers that already exist are skipped."""
        full = f"correction/generated/{self.name}/{case_id}"
        if full in self.ids:
            return
        self.ids.add(full)
        out = self.lines
        out.append("[[case]]")
        out.append(f"id = {toml_string(full)}")
        out.append('kind = "correction"')
        out.append('class = "mandatory"')
        out.append('language = "1.0"')
        if requires:
            out.append("requires = [" + ", ".join(toml_string(r) for r in requires) + "]")
        out.append(f"summary = {toml_string(summary)}")
        out.append('method = "correct"')
        out.append(f"program = {toml_string(program)}")
        out.append("")
        if error is not None:
            out.append("[case.expect.error]")
            out.append(f"code = {toml_string(error)}")
            if candidate is not None:
                out.append(f"candidates = [{{ program = {toml_string(candidate)} }}]")
        else:
            out.append("[case.expect]")
            out.append(f"program = {toml_string(expect_program or program)}")
            if corrections is not None:
                items = []
                for c in corrections:
                    fields = ", ".join(f"{k} = {toml_string(v)}" for k, v in c.items())
                    items.append("{ " + fields + " }")
                out.append("corrections = [" + ", ".join(items) + "]")
        out.append("")

    def render(self) -> str:
        """Returns the text of the file."""
        head = [
            f"# {self.summary}",
            "# Written by conformance/tools/generate_correction.py; edit the generator, not this file.",
            "",
            'format = "nurt-conformance-cases/1"',
            "",
        ]
        return "\n".join(head + self.lines).rstrip("\n") + "\n"


def requires_for(command: suite.Command) -> tuple[str, ...]:
    """Returns the requirements of a case on a command restricted to some systems."""
    return tuple(f"os:{o}" for o in command.os)


def seed(command: str, written: str | None = None) -> str:
    """Returns the seed program of a command with the stage name as written."""
    return SEEDS[command].replace("{cmd}", written or command)


def command_typos(catalog: dict[str, suite.Command]) -> Writer:
    """Typing errors in command names that the correction must fix."""
    w = Writer("command-typos", "Typing errors in command names corrected by distance.")
    for name in sorted(SEEDS):
        cmd = catalog[name]
        accepted = []
        for typo in edits(name):
            r = suite.resolve_command(typo, catalog)
            if r.outcome == "fuzzy" and r.name == name and r.distance == 1:
                accepted.append(typo)
        for typo in accepted[:3]:
            w.add(
                f"{slug(name)}/{slug(typo)}",
                f"The command name {typo} is one edit from {name} and from no other name.",
                seed(name, typo),
                seed(name),
                [{"rule": "L01", "class": "F", "from": typo, "to": name}],
                requires=requires_for(cmd),
            )
        if len(name) >= 8 and not cmd.changing:
            for found in two_edits(name):
                r = suite.resolve_command(found, catalog)
                if r.outcome == "fuzzy" and r.name == name and r.distance == 2:
                    w.add(
                        f"{slug(name)}/{slug(found)}",
                        f"The command name {found} is two edits from {name}, a long name of a reading command.",
                        seed(name, found),
                        seed(name),
                        [{"rule": "L01", "class": "F", "from": found, "to": name}],
                        requires=requires_for(cmd),
                    )
                    break
    return w


def camel(name: str) -> str:
    """Returns a snake case name in camel case."""
    head, *rest = name.split("_")
    return head + "".join(p.capitalize() for p in rest)


def pascal(name: str) -> str:
    """Returns a snake case name in Pascal case."""
    return "".join(p.capitalize() for p in name.split("_"))


def conventions(catalog: dict[str, suite.Command]) -> Writer:
    """Names written in another letter case or naming convention."""
    w = Writer("conventions", "Names in another letter case or naming convention.")
    for name in sorted(SEEDS):
        cmd = catalog[name]
        upper = name.upper()
        w.add(
            f"{slug(name)}/upper",
            f"The command name {name} in upper case.",
            seed(name, upper),
            seed(name),
            [{"rule": "L03", "class": "C", "from": upper, "to": name}],
            requires=requires_for(cmd),
        )
        parts = name.split(".")
        leaf = parts[-1]
        if "_" in leaf:
            prefix = ".".join(parts[:-1] + [""])
            for style, written in (("kebab", leaf.replace("_", "-")), ("camel", camel(leaf))):
                full = prefix + written
                w.add(
                    f"{slug(name)}/{style}",
                    f"The command name {name} in {style} case.",
                    seed(name, full),
                    seed(name),
                    [{"rule": "L04", "class": "C", "from": full, "to": name}],
                    requires=requires_for(cmd),
                )
        for param in parameter_candidates(cmd):
            value = param.default
            base = seed(name)
            if "_" in param.name:
                for style, written in (
                    ("camel", camel(param.name)),
                    ("kebab", param.name.replace("_", "-")),
                    ("pascal", pascal(param.name)),
                ):
                    w.add(
                        f"{slug(name)}/param-{slug(param.name)}-{style}",
                        f"The parameter {param.name} of {name} in {style} case.",
                        f"{base} {written}: {value}",
                        f"{base} {param.name}: {value}",
                        [{"rule": "L04", "class": "C", "from": written, "to": param.name}],
                        requires=requires_for(cmd),
                    )
            else:
                written = param.name.capitalize()
                w.add(
                    f"{slug(name)}/param-{slug(param.name)}-capital",
                    f"The parameter {param.name} of {name} with a capital letter.",
                    f"{base} {written}: {value}",
                    f"{base} {param.name}: {value}",
                    [{"rule": "L03", "class": "C", "from": written, "to": param.name}],
                    requires=requires_for(cmd),
                )
    return w


def parameter_candidates(cmd: suite.Command) -> list[suite.Param]:
    """Returns the named parameters with a default that the seed of a command can take."""
    if cmd.name in SKIPPED_PARAMS:
        return []
    written = SEEDS[cmd.name].split("{cmd}", 1)[1]
    out = []
    for p in cmd.params.values():
        if p.position is not None or p.default is None:
            continue
        if f"{p.name}:" in written:
            continue
        if p.default in ("all",) or p.default.startswith("[") and p.default != "[]":
            continue
        out.append(p)
    return out


def parameter_typos(catalog: dict[str, suite.Command]) -> Writer:
    """Typing errors and prefixes of parameter names."""
    w = Writer("parameter-typos", "Typing errors and unique prefixes of parameter names.")
    for name in sorted(SEEDS):
        cmd = catalog[name]
        names = list(cmd.params)
        base = seed(name)
        taken = 0
        for p in parameter_candidates(cmd):
            if taken >= 4:
                break
            if len(p.name) < 4:
                continue
            for typo in edits(p.name):
                r = suite.resolve_name(typo, names)
                if r.outcome == "fuzzy" and r.name == p.name:
                    w.add(
                        f"{slug(name)}/{slug(p.name)}-{slug(typo)}",
                        f"The parameter name {typo} is within the distance threshold of {p.name} only.",
                        f"{base} {typo}: {p.default}",
                        f"{base} {p.name}: {p.default}",
                        [{"rule": "L02", "class": "F", "from": typo, "to": p.name}],
                        requires=requires_for(cmd),
                    )
                    taken += 1
                    break
            if len(p.name) >= 7:
                for size in range(3, len(p.name) - 3):
                    prefix = p.name[:size]
                    if prefix.endswith("_"):
                        continue
                    matches = [n for n in names if n.startswith(prefix)]
                    if matches == [p.name] and prefix not in names:
                        w.add(
                            f"{slug(name)}/{slug(p.name)}-prefix",
                            f"The prefix {prefix} names only the parameter {p.name}.",
                            f"{base} {prefix}: {p.default}",
                            f"{base} {p.name}: {p.default}",
                            [{"rule": "L05", "class": "F", "from": prefix, "to": p.name}],
                            requires=requires_for(cmd),
                        )
                        break
    return w


def value_typos(catalog: dict[str, suite.Command]) -> Writer:
    """Typing errors in values of enumerations."""
    w = Writer("value-typos", "Typing errors in enumeration values.")
    for name in sorted(SEEDS):
        cmd = catalog[name]
        base = seed(name)
        for p in parameter_candidates(cmd):
            if not p.values or p.default not in p.values:
                continue
            target = p.default
            if len(target) < 4:
                continue
            for typo in edits(target):
                r = suite.resolve_name(typo, list(p.values))
                if r.outcome == "fuzzy" and r.name == target:
                    w.add(
                        f"{slug(name)}/{slug(p.name)}-{slug(typo)}",
                        f"The value {typo} of {p.name} is within the distance threshold of {target} only.",
                        f"{base} {p.name}: {typo}",
                        f"{base} {p.name}: {target}",
                        [{"rule": "V01", "class": "F", "from": typo, "to": target}],
                        requires=requires_for(cmd),
                    )
                    break
    return w


def adversarial(catalog: dict[str, suite.Command]) -> Writer:
    """Inputs close to a correction where the right outcome is no correction."""
    w = Writer("adversarial", "Inputs close to a correction where no name may be guessed.")
    for name in sorted(SEEDS):
        cmd = catalog[name]
        req = requires_for(cmd)
        ties = []
        far = []
        for typo in edits(name):
            r = suite.resolve_command(typo, catalog)
            if r.outcome == "tie" and name in r.tied and all(t in catalog for t in r.tied):
                ties.append((typo, r.tied))
            elif r.outcome == "none" and r.distance >= 1:
                far.append(typo)
        for typo, tied in ties[:2]:
            others = ", ".join(t for t in tied if t != name)
            w.add(
                f"tie/{slug(name)}-{slug(typo)}",
                f"The name {typo} is equally close to {name} and {others}; no command is chosen.",
                seed(name, typo),
                error="E_UNKNOWN_COMMAND",
                requires=req,
            )
        for typo in far[:1]:
            w.add(
                f"threshold/{slug(name)}-{slug(typo)}",
                f"The name {typo} is outside the distance threshold of {name}.",
                seed(name, typo),
                error="E_UNKNOWN_COMMAND",
                requires=req,
            )
        if 4 <= len(name) <= 7:
            for found in two_edits(name):
                r = suite.resolve_command(found, catalog)
                if r.outcome == "none" and r.tied == (name,):
                    w.add(
                        f"threshold/{slug(name)}-{slug(found)}",
                        f"The name {found} is two edits from {name}, beyond the threshold of a name of {len(name)} characters.",
                        seed(name, found),
                        error="E_UNKNOWN_COMMAND",
                        requires=req,
                    )
                    break
        swapped = None
        for c in "qxz":
            candidate = c + name[1:]
            r = suite.resolve_command(candidate, catalog)
            if r.outcome == "none" and r.tied == (name,):
                swapped = candidate
                break
        if swapped:
            w.add(
                f"first-character/{slug(name)}",
                f"The name {swapped} differs from {name} in the first character, which is never corrected.",
                seed(name, swapped),
                error="E_UNKNOWN_COMMAND",
                requires=req,
            )
        if len(name) >= 8 and cmd.changing:
            for found in two_edits(name):
                r = suite.resolve_command(found, catalog)
                if r.outcome == "ambiguous" and r.name == name:
                    w.add(
                        f"changing-distance-two/{slug(name)}-{slug(found)}",
                        f"The name {found} is two edits from the changing command {name}; the correction is offered, not applied.",
                        seed(name, found),
                        error="E_AMBIGUOUS",
                        candidate=seed(name),
                        requires=req,
                    )
                    break
        typo = next(
            (t for t in edits(name) if suite.resolve_command(t, catalog).outcome == "fuzzy"),
            None,
        )
        if typo is not None:
            leaf = typo.replace(".", "-")
            for label, program in (
                ("path", f'read "/home/user/notes/{leaf}.txt"'),
                ("text", f'search "{typo}" "/home/user/app"'),
                ("write", f'write "/home/user/notes/{leaf}.txt" "{typo}\\n" mode: create'),
            ):
                w.add(
                    f"literal/{label}-{slug(typo)}",
                    f"The name {typo} inside a literal is content and stays as written.",
                    program,
                )
    return w


def build() -> dict[Path, str]:
    """Returns the generated files with their text."""
    catalog = suite.load_catalog()
    missing = sorted(set(catalog) - set(SEEDS))
    if missing:
        raise SystemExit(f"no seed program for: {', '.join(missing)}")
    writers = [
        command_typos(catalog),
        conventions(catalog),
        parameter_typos(catalog),
        value_typos(catalog),
        adversarial(catalog),
    ]
    return {OUT / f"{w.name}.cases.toml": w.render() for w in writers}


def body(text: str) -> str:
    """Returns the text without the leading comment block added to every file."""
    lines = text.splitlines()
    i = 0
    while i < len(lines) and (lines[i].startswith("#") or not lines[i].strip()):
        i += 1
    return "\n".join(lines[i:])


def main(argv: list[str]) -> int:
    """Writes or compares the generated files."""
    files = build()
    if "--check" in argv:
        stale = [
            str(p.relative_to(suite.REPO))
            for p, text in files.items()
            if not p.exists() or body(p.read_text(encoding="utf-8")) != body(text)
        ]
        if stale:
            print("generated files are out of date: " + ", ".join(stale), file=sys.stderr)
            return 1
        return 0
    OUT.mkdir(parents=True, exist_ok=True)
    for p, text in files.items():
        p.write_text(text, encoding="utf-8", newline="\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
