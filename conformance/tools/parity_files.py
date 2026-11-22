# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Parity scenarios of the file tools: reading, searching, finding, writing and editing.

Expected rows of searches and listings are computed from the initial tree by a
small reference model (regular expressions, glob patterns, byte order of
paths); the expected trees after a change are computed by plain text
operations. Every scenario is a program run against a real directory.
"""

from __future__ import annotations

import gzip
import json
import re

from parity_model import (
    REMOVED, UNCHANGED, Collection, changed, entry, records, row, step, tree,
)

TEXT = "one\ntwo\nthree\nfour\nfive\n"
LINES = TEXT.splitlines()

SOURCES = {
    "src/a.go": "package a\n\nfunc oldName() {}\n// TODO: rename oldName\n",
    "src/b.go": "package b\n\nfunc use() { oldName() }\n",
    "src/c.txt": "todo list\nTODO later\n",
    "src/deep/d.go": "package deep\n\nvar Old = 1\n",
    "docs/readme.md": "# Readme\n\nTODO docs\n",
    ".hidden/x.go": "// TODO hidden\n",
    "vendor/v.go": "// TODO vendored\n",
}


def glob_regex(pattern: str) -> re.Pattern:
    """Returns the regular expression of a glob relative to the searched directory."""
    out = ""
    i = 0
    while i < len(pattern):
        if pattern.startswith("**/", i):
            out += "(?:.*/)?"
            i += 3
        elif pattern.startswith("**", i):
            out += ".*"
            i += 2
        elif pattern[i] == "*":
            out += "[^/]*"
            i += 1
        elif pattern[i] == "?":
            out += "[^/]"
            i += 1
        else:
            out += re.escape(pattern[i])
            i += 1
    return re.compile(out + r"\Z")


def hidden(path: str) -> bool:
    """Whether a relative path has a hidden component."""
    return any(part.startswith(".") for part in path.split("/"))


def text_files(fs: dict) -> list[str]:
    """Returns the paths of the text files of a tree in byte order."""
    return sorted((p for p, c in fs.items() if isinstance(c, str)), key=lambda p: p.encode())


def matches(fs: dict, pattern: str, *, flags: int = 0, glob: str = "**", show_hidden: bool = True,
            invert: bool = False, word: bool = False, under: str = "") -> list[dict]:
    """Returns the expected match records of a line search."""
    rx = re.compile(rf"\b(?:{pattern})\b" if word else pattern, flags)
    g = glob_regex(glob)
    rows = []
    for path in text_files(fs):
        if under and not path.startswith(under + "/"):
            continue
        rel = path[len(under) + 1:] if under else path
        if not g.match(rel) or (not show_hidden and hidden(rel)):
            continue
        for no, line in enumerate(fs[path].splitlines(), 1):
            found = rx.search(line)
            if invert and found is None:
                rows.append(row("match", path=path, no=no, line=line))
            elif not invert and found is not None:
                rows.append(row("match", path=path, no=no, text=found.group(0), line=line))
    return rows


def found(fs: dict, glob: str, *, kind: str = "file", show_hidden: bool = True,
          depth: int | None = None, under: str = "") -> list[dict]:
    """Returns the expected entry records of find."""
    paths: set[str] = set()
    for path, content in fs.items():
        parts = path.split("/")
        for i in range(1, len(parts) + 1):
            sub = "/".join(parts[:i])
            is_file = i == len(parts) and content is not None
            paths.add(("f:" if is_file else "d:") + sub)
    g = glob_regex(glob)
    rows = []
    for item in sorted(paths, key=lambda p: p[2:].encode()):
        t, path = item[:2], item[2:]
        if under:
            if not path.startswith(under + "/"):
                continue
            rel = path[len(under) + 1:]
        else:
            rel = path
        if depth is not None and rel.count("/") + 1 > depth:
            continue
        if not show_hidden and hidden(rel):
            continue
        if kind == "file" and t != "f:" or kind == "directory" and t != "d:":
            continue
        if g.match(rel):
            rows.append(row("entry", path=path, name=path.rsplit("/", 1)[-1],
                            type="file" if t == "f:" else "directory"))
    return rows


def read_scenarios() -> Collection:
    """Reading files: ranges, encodings, line endings, documents and errors."""
    c = Collection("files", "read")
    fs = tree({"a.txt": TEXT})
    c.add("whole-file", "Reads a whole UTF-8 file.", fs, 'read "a.txt"',
          report={"status": "ok", "kind": "read",
                  "records": records("content", [row("content", path="a.txt", text=TEXT, lines=5)])})
    for name, rng, lo, hi in (
        ("range-middle", "2..3", 2, 3), ("range-open-start", "..2", 1, 2),
        ("range-open-end", "4..", 4, 5), ("range-single-line", "3", 3, 3),
        ("range-whole", "1..5", 1, 5), ("range-past-end", "4..9", 4, 5),
    ):
        text = "".join(line + "\n" for line in LINES[lo - 1:hi])
        c.add(name, f"Reads the lines {rng} of a file.", fs, f'read "a.txt" lines: {rng}',
              report={"status": "ok", "records": records("content", [row("content", path="a.txt", text=text)])})
    for n, line in enumerate(LINES, 1):
        c.add(f"line-{n}", f"Reads line {n} as a range of one line.", fs, f'read "a.txt" lines: {n}..{n}',
              report={"status": "ok", "records": records("content", [row("content", path="a.txt", text=line + "\n")])})
    for n in (1, 2, 5):
        text = "".join(line + "\n" for line in LINES[-n:])
        c.add(f"last-{n}", f"Reads the last {n} lines of a file.", fs, f'read "a.txt" last: {n}',
              report={"status": "ok", "records": records("content", [row("content", path="a.txt", text=text)])})
    for ctx, lo, hi in ((0, 3, 3), (1, 2, 4), (2, 1, 5)):
        text = "".join(line + "\n" for line in LINES[lo - 1:hi])
        c.add(f"around-context-{ctx}", f"Reads the lines around a match with {ctx} lines of context.", fs,
              f'read "a.txt" around: /three/ context: {ctx}',
              report={"status": "ok", "records": records("content", [row("content", path="a.txt", text=text)])})
    c.add("several-ranges", "Reads two ranges of one file in one record.", fs, 'read "a.txt" lines: [1..1, 5..5]',
          report={"status": "ok", "records": records("content", matched=1)})
    c.add("line-numbers", "Reads with line numbers.", fs, 'read "a.txt" lines: 1..2 line_numbers: true',
          report={"status": "ok", "records": records("content", matched=1)})
    c.add("missing-file", "Reading a missing file fails and changes nothing.", fs, 'read "missing.txt"',
          report={"status": "failed", "error": {"code": "E_NOT_FOUND"}})
    c.add("empty-file", "Reads an empty file.", tree({"e.txt": ""}), 'read "e.txt"',
          report={"status": "ok", "records": records("content", [row("content", path="e.txt", text="", lines=0)])})
    c.add("no-final-newline", "Reads a file without a final line ending.", tree({"a.txt": "a\nb"}), 'read "a.txt"',
          report={"status": "ok", "records": records("content", [row("content", path="a.txt", text="a\nb", lines=2)])})
    c.add("crlf", "Reports CRLF line endings of a file.", tree({"w.txt": "a\r\nb\r\n"}), 'read "w.txt"',
          report={"status": "ok", "records": records("content", [row("content", path="w.txt", line_ending="crlf", lines=2)])})
    c.add("mixed-line-endings", "Reports mixed line endings.", tree({"m.txt": "a\r\nb\nc\r\n"}), 'read "m.txt"',
          report={"status": "ok", "records": records("content", [row("content", path="m.txt", line_ending="mixed")])})
    legacy = "Škoda Žilina\n"
    c.add("cp1250", "Detects and decodes a Windows-1250 file.", tree({"l.txt": legacy.encode("cp1250")}), 'read "l.txt"',
          report={"status": "ok", "records": records("content", [row("content", path="l.txt", text=legacy, encoding="windows-1250")])})
    c.add("cp852-explicit", "Decodes an IBM 852 file with the given encoding.", tree({"d.txt": legacy.encode("cp852")}),
          'read "d.txt" encoding: "ibm852"',
          report={"status": "ok", "records": records("content", [row("content", path="d.txt", text=legacy)])})
    wide = "héllo wörld\n"
    c.add("utf16-bom", "Decodes a UTF-16 file with a byte order mark.", tree({"u.txt": b"\xff\xfe" + wide.encode("utf-16-le")}),
          'read "u.txt"', report={"status": "ok", "records": records("content", [row("content", path="u.txt", text=wide, bom=True)])})
    c.add("utf8-bom", "Reads a UTF-8 file with a byte order mark; the mark is not part of the text.",
          tree({"b.txt": b"\xef\xbb\xbfhello\n"}), 'read "b.txt"',
          report={"status": "ok", "records": records("content", [row("content", path="b.txt", text="hello\n", bom=True)])})
    c.add("gzip", "Reads the text of a gzip-compressed file transparently.",
          tree({"a.log.gz": gzip.compress(b"line 1\nline 2\n", mtime=0)}), 'read "a.log.gz"',
          report={"status": "ok", "records": records("content", [row("content", text="line 1\nline 2\n")]),
                  "notable": [{"code": "decompressed"}]})
    c.add("gzip-raw", "Reads a compressed file without decompression when asked.",
          tree({"a.log.gz": gzip.compress(b"x\n", mtime=0)}), 'read "a.log.gz" decompress: none',
          report={"status": "ok"})
    two = tree({"a.txt": "a\n", "b.txt": "b\n"})
    c.add("several-files", "Reads several files, one record each.", two, 'read ["a.txt", "b.txt"]',
          report={"status": "ok", "records": records("content", [row("content", path="a.txt", text="a\n"),
                                                               row("content", path="b.txt", text="b\n")])})
    c.add("stage-of-find", "Reads the files found by find.", tree({"a.md": "a\n", "b.md": "b\n", "c.txt": "c\n"}),
          'find "." glob: "*.md" depth: 1 |> read',
          report={"status": "ok", "records": records("content", [row("content", path="a.md"), row("content", path="b.md")])})
    doc = json.dumps({"name": "app", "version": "1.2.0", "scripts": {"test": "vitest"}}, indent=2) + "\n"
    c.add("json-key", "Reads one key of a JSON document.", tree({"package.json": doc}), 'read "package.json" key: "scripts.test"',
          report={"status": "ok", "records": records("item", matched=1)})
    c.add("json-missing-key", "Reading a missing key fails.", tree({"package.json": doc}), 'read "package.json" key: "scripts.build"',
          report={"status": "failed", "error": {"code": "E_KEY_NOT_FOUND"}})
    md = "# App\n\n## Install\n\nRun make.\n\n## Usage\n\nRun app.\n"
    c.add("markdown-section", "Reads one section of a Markdown file.", tree({"README.md": md}), 'read "README.md" section: "Install"',
          report={"status": "ok", "records": records("content", matched=1)})
    c.add("unchanged-since", "A second read with the entity tag of the first reports the file unchanged.", fs,
          steps=[step('read "a.txt"', report={"status": "ok"}),
                 step('read "a.txt" unchanged_since: "${step1/report/records/rows/0/fields/etag/v}"',
                      report={"status": "ok", "records": records("content", [row("content", path="a.txt", unchanged=True)])},
                      after=UNCHANGED)])
    c.add("changed-since", "A read after a change with the earlier entity tag returns the new content.", fs,
          steps=[step('read "a.txt"', report={"status": "ok"}),
                 step('write "a.txt" "new\\n" mode: overwrite', report={"status": "ok"}),
                 step('read "a.txt" unchanged_since: "${step1/report/records/rows/0/fields/etag/v}"',
                      report={"status": "ok", "records": records("content", [row("content", path="a.txt", text="new\n")])})])
    c.add("lines-records", "Produces one record per line.", fs, 'lines "a.txt" 2..3',
          report={"status": "ok", "records": records("line", [row("line", path="a.txt", no=2, text="two"),
                                                            row("line", path="a.txt", no=3, text="three")])})
    c.add("lines-filtered", "Filters lines by a regular expression.", fs, 'lines "a.txt" |> filter text ~ /^t/',
          report={"status": "ok", "records": records("line", [row("line", no=2, text="two"), row("line", no=3, text="three")])})
    c.add("lines-count", "Counts the lines of a file.", fs, 'lines "a.txt" |> count', report={"status": "ok"})
    c.add("long-line-limit", "Shortens long lines to the given limit.", tree({"l.txt": "x" * 300 + "\n"}),
          'read "l.txt" max_line: 100', report={"status": "ok", "records": records("content", matched=1)})
    return c


def search_scenarios() -> Collection:
    """Searching file content with the options of a content search."""
    c = Collection("files", "search")
    fs = tree(SOURCES)

    def add(name: str, summary: str, program: str, rows: list[dict], **extra: object) -> None:
        report = {"status": "ok", "kind": "search", "records": records("match", rows)}
        report.update(extra)
        c.add(name, summary, fs, program, report=report)

    add("literal", "Finds a text in every file under a directory.", 'search "TODO" "."', matches(SOURCES, "TODO"))
    add("literal-in-subdirectory", "Searches one subdirectory.", 'search "TODO" "src"', matches(SOURCES, "TODO", under="src"))
    add("regex", "Finds a regular expression.", r'search /old[A-Z]\w+/ "src"', matches(SOURCES, r"old[A-Z]\w+", under="src"))
    add("regex-anchored", "Finds lines that start with a pattern.", 'search /^package/ "src"', matches(SOURCES, "^package", under="src"))
    add("case-insensitive", "Finds a text without letter case.", 'search "todo" "src" case_sensitive: false',
        matches(SOURCES, "todo", flags=re.I, under="src"))
    add("case-sensitive-default", "Letter case matters by default.", 'search "todo" "src"', matches(SOURCES, "todo", under="src"))
    add("whole-word", "Finds whole words only.", 'search "old" "src" whole_word: true case_sensitive: false',
        matches(SOURCES, "old", flags=re.I, word=True, under="src"))
    add("glob", "Limits the search to files matching a glob.", 'search "TODO" "." glob: "**/*.go"',
        matches(SOURCES, "TODO", glob="**/*.go"))
    add("glob-list", "Limits the search to files matching any glob of a list.", 'search "TODO" "." glob: ["**/*.md", "**/*.txt"]',
        [r for r in matches(SOURCES, "TODO") if r["fields"]["path"]["v"].endswith((".md", ".txt"))])
    add("without-hidden", "Skips hidden directories when asked.", 'search "TODO" "." hidden: false',
        matches(SOURCES, "TODO", show_hidden=False))
    add("exclude", "Skips files matching an exclusion.", 'search "TODO" "." exclude: ["vendor/**"]',
        [r for r in matches(SOURCES, "TODO") if "/vendor/" not in r["fields"]["path"]["v"]])
    add("no-match", "A search without matches returns no records and succeeds.", 'search "absent-text" "."', [])
    add("several-files-as-path", "Searches a list of files.", 'search "package" ["src/a.go", "src/b.go"]',
        matches({k: v for k, v in SOURCES.items() if k in ("src/a.go", "src/b.go")}, "package"))
    add("pattern-list", "Finds any pattern of a list.", 'search ["TODO", "package"] "src/a.go"',
        matches({"src/a.go": SOURCES["src/a.go"]}, "TODO|package"))
    add("special-characters-as-text", "A text pattern with regular expression characters is matched literally.",
        'search "oldName()" "src"', matches(SOURCES, re.escape("oldName()"), under="src"))
    add("literal-mode", "mode literal matches a regular expression text literally.", 'search "(" "src" mode: literal',
        matches(SOURCES, re.escape("("), under="src"))
    inverted = matches({"src/c.txt": SOURCES["src/c.txt"]}, "later", invert=True)
    add("invert", "Returns the lines that do not match.", 'search "later" "src/c.txt" invert: true', inverted)
    c.add("output-files", "Returns the matching files instead of the matches.", fs, 'search "TODO" "src" output: files',
          report={"status": "ok", "records": records("entry", [row("entry", path="src/a.go"), row("entry", path="src/c.txt")])})
    c.add("output-counts", "Returns the number of matches per file.", fs, 'search "oldName" "src" output: counts',
          report={"status": "ok", "records": records("match", [row("match", path="src/a.go", count=2),
                                                             row("match", path="src/b.go", count=1)])})
    c.add("output-unmatched", "Returns the files without a match.", fs, 'search "TODO" "src" glob: "**/*.go" output: unmatched',
          report={"status": "ok", "records": records("entry", [row("entry", path="src/b.go"), row("entry", path="src/deep/d.go")])})
    c.add("max-per-file", "Limits the matches per file.", tree({"m.txt": "x1\nx2\nx3\n"}), 'search "x" "m.txt" max_per_file: 2',
          report={"status": "ok", "records": records("match", matched=2)})
    ctx = tree({"c.txt": "a\nb\nTARGET\nd\ne\n"})
    for name, args, before, after in (
        ("context", "context: 1", ["b"], ["d"]), ("before", "before: 2", ["a", "b"], []),
        ("after", "after: 2", [], ["d", "e"]), ("context-two", "context: 2", ["a", "b"], ["d", "e"]),
    ):
        fields = {"no": 3, "text": "TARGET"}
        if before:
            fields["before"] = before
        if after:
            fields["after"] = after
        c.add(name, f"Returns lines of context with {args}.", ctx, f'search "TARGET" "c.txt" {args}',
              report={"status": "ok", "records": records("match", [row("match", **fields)])})
    c.add("only-matching", "Returns only the matched text.", fs, r'search /old\w+/ "src/b.go" only_matching: true',
          report={"status": "ok", "records": records("match", [row("match", path="src/b.go", no=3, text="oldName")])})
    c.add("binary-skipped", "Skips binary files and reports how many.", tree({"t.txt": "TODO\n", "b.bin": b"\x00TODO\x00\n"}),
          'search "TODO" "."', report={"status": "ok", "records": records("match", [row("match", path="t.txt")]),
                                     "notable": [{"code": "binary_skipped", "count": 1}]})
    c.add("binary-as-text", "Searches binary files as text when asked.", tree({"b.bin": b"\x00TODO\x00\n"}),
          'search "TODO" "." binary: text', report={"status": "ok", "records": records("match", matched=1)})
    c.add("multiline", "Matches across lines when asked.", tree({"m.txt": "begin\nmiddle\nend\n"}),
          r'search /begin\nmiddle/ "m.txt" multiline: true', report={"status": "ok", "records": records("match", matched=1)})
    c.add("gitignore", "Skips files ignored by .gitignore when asked.",
          tree({".gitignore": "build/\n", "build/x.txt": "TODO\n", "src/y.txt": "TODO\n"}),
          'search "TODO" "." gitignore: true',
          report={"status": "ok", "records": records("match", [row("match", path="src/y.txt")])})
    c.add("exclude-preset-deps", "Skips dependency directories with a preset.",
          tree({"node_modules/p/i.js": "TODO\n", "src/i.js": "TODO\n"}), 'search "TODO" "." exclude_preset: [deps]',
          report={"status": "ok", "records": records("match", [row("match", path="src/i.js")])})
    c.add("trim", "Trims leading white space of the returned lines.", tree({"t.py": "    TODO here\n"}),
          'search "TODO" "t.py" trim: true',
          report={"status": "ok", "records": records("match", [row("match", line="TODO here")])})
    c.add("dedupe", "Returns repeated lines once.", tree({"d.txt": "same TODO\nsame TODO\n"}), 'search "TODO" "d.txt" dedupe: true',
          report={"status": "ok", "records": records("match", matched=1)})
    c.add("crlf-file", "Matches lines of a CRLF file without the line ending.", tree({"w.txt": "a TODO\r\nb\r\n"}),
          'search "TODO" "w.txt"', report={"status": "ok", "records": records("match", [row("match", no=1, line="a TODO")])})
    c.add("legacy-encoding", "Searches a Windows-1250 file by its decoded text.",
          tree({"l.txt": "Žilina\nBrno\n".encode("cp1250")}), 'search "Žilina" "l.txt"',
          report={"status": "ok", "records": records("match", [row("match", no=1, text="Žilina")])})
    c.add("utf16-file", "Searches a UTF-16 file by its decoded text.",
          tree({"u.txt": b"\xff\xfe" + "alpha\nbeta\n".encode("utf-16-le")}), 'search "beta" "u.txt"',
          report={"status": "ok", "records": records("match", [row("match", no=2, text="beta")])})
    c.add("missing-path", "Searching a missing path fails.", fs, 'search "TODO" "absent"',
          report={"status": "failed", "error": {"code": "E_NOT_FOUND"}})
    c.add("invalid-regex", "An invalid regular expression is rejected before reading.", fs, 'search /(unclosed/ "src"',
          report={"status": "failed", "error": {"code": "E_REGEX"}})
    c.add("count-matches", "Counts the matches of a search.", fs, 'search "TODO" "." |> count', report={"status": "ok"})
    c.add("stage-on-found-files", "Searches the files found by find.", fs, 'find "src" glob: "**/*.go" |> search "oldName"',
          report={"status": "ok", "records": records("match", matches(SOURCES, "oldName", glob="**/*.go", under="src"))})
    c.add("stage-on-lines", "Searches the lines of one file.", tree({"a.txt": TEXT}), 'lines "a.txt" |> search "o"',
          report={"status": "ok", "records": records("match", matched=3)})
    c.add("sort-by-count", "Sorts files by the number of matches.", fs, 'search "oldName" "src" output: counts sort: count',
          report={"status": "ok", "records": records("match", matched=2)})
    c.add("file-type", "Limits the search to a file type.", fs, 'search "TODO" "." file_type: ["go"]',
          report={"status": "ok", "records": records("match", matches(SOURCES, "TODO", glob="**/*.go"))})
    c.add("exclude-file-type", "Skips a file type.", fs, 'search "TODO" "src" exclude_file_type: ["go"]',
          report={"status": "ok", "records": records("match", [row("match", path="src/c.txt")])})
    c.add("whitespace-normalize", "Matches a text with different runs of white space.",
          tree({"s.txt": "a   b\tc\n"}), 'search "a b c" "s.txt" whitespace: normalize',
          report={"status": "ok", "records": records("match", matched=1)})
    c.add("named-groups", "Returns named groups of a regular expression.", tree({"l.log": "ERROR: disk full\n"}),
          r'search /(?P<level>[A-Z]+): (?P<msg>.*)/ "l.log"', report={"status": "ok", "records": records("match", matched=1)})
    c.add("search-then-first", "Keeps the first match only.", fs, 'search "TODO" "." |> first 1',
          report={"status": "ok", "records": records("match", matches(SOURCES, "TODO")[:1])})
    return c


def find_scenarios() -> Collection:
    """Finding and listing entries and describing paths."""
    c = Collection("files", "find")
    layout = {
        "a.go": "package a\n", "b.txt": "b\n", ".env": "X=1\n", "src/c.go": "package c\n",
        "src/d/e.go": "package e\n", "src/d/f.md": "# f\n", "docs/g.md": "# g\n", "empty": None,
    }
    fs = tree(layout)
    for name, program, kw in (
        ("all-files", 'find "."', dict(glob="**")),
        ("glob-recursive", 'find "." glob: "**/*.go"', dict(glob="**/*.go")),
        ("glob-top-level", 'find "." glob: "*.go" depth: 1', dict(glob="*.go", depth=1)),
        ("glob-in-subdirectory", 'find "src" glob: "**/*.md"', dict(glob="**/*.md", under="src")),
        ("glob-question-mark", 'find "." glob: "?.txt" depth: 1', dict(glob="?.txt", depth=1)),
        ("glob-list", 'find "." glob: ["**/*.go", "**/*.md"]', None),
        ("depth-two", 'find "." depth: 2', dict(glob="**", depth=2)),
        ("directories", 'find "." type: directory', dict(glob="**", kind="directory")),
        ("without-hidden", 'find "." hidden: false', dict(glob="**", show_hidden=False)),
        ("path-glob", 'find "." glob: "src/**/*.go"', dict(glob="src/**/*.go")),
    ):
        if kw is None:
            rows = sorted(found(layout, "**/*.go") + found(layout, "**/*.md"),
                          key=lambda r: r["fields"]["path"]["v"].encode())
        else:
            rows = found(layout, **kw)
        c.add(name, f"Finds entries with {program[5:].strip() or 'defaults'}.", fs, program,
              report={"status": "ok", "kind": "find", "records": records("entry", rows)})
    for depth in (1, 2, 3):
        for kind in ("file", "directory"):
            c.add(f"depth-{depth}-{kind}", f"Finds entries of type {kind} up to depth {depth}.", fs,
                  f'find "." type: {kind} depth: {depth}',
                  report={"status": "ok", "records": records("entry", found(layout, "**", kind=kind, depth=depth))})
    c.add("any-type", "Finds files and directories.", fs, 'find "src" type: any',
          report={"status": "ok", "records": records("entry", matched=4)})
    c.add("exclude", "Skips entries matching an exclusion.", fs, 'find "." glob: "**/*.go" exclude: ["src/d/**"]',
          report={"status": "ok", "records": records("entry", [row("entry", path="a.go"), row("entry", path="src/c.go")])})
    c.add("exclude-preset-vcs", "Skips version control directories with a preset.",
          tree({".git/config": "[core]\n", "a.txt": "a\n"}), 'find "." exclude_preset: [vcs]',
          report={"status": "ok", "records": records("entry", [row("entry", path="a.txt")])})
    c.add("gitignore", "Skips ignored entries when asked.", tree({".gitignore": "*.log\n", "a.log": "x\n", "b.txt": "y\n"}),
          'find "." gitignore: true hidden: false', report={"status": "ok", "records": records("entry", [row("entry", path="b.txt")])})
    c.add("case-insensitive", "Matches names without letter case when asked.", tree({"A.GO": "x\n", "b.go": "y\n"}),
          'find "." glob: "*.go" depth: 1 case_sensitive: false',
          report={"status": "ok", "records": records("entry", [row("entry", path="A.GO"), row("entry", path="b.go")])})
    c.add("case-sensitive-default", "Matches names with letter case by default.", tree({"A.GO": "x\n", "b.go": "y\n"}),
          'find "." glob: "*.go" depth: 1', report={"status": "ok", "records": records("entry", [row("entry", path="b.go")])})
    sized = tree({"s.txt": "x", "m.txt": "x" * 10, "l.txt": "x" * 100})
    c.add("sort-size-desc", "Sorts entries by size, largest first.", sized, 'find "." sort: size order: desc',
          report={"status": "ok", "records": records("entry", [row("entry", name="l.txt", size=100),
                                                             row("entry", name="m.txt", size=10),
                                                             row("entry", name="s.txt", size=1)])})
    c.add("filter-size", "Keeps entries larger than a size.", sized, 'find "." |> filter size > 5b',
          report={"status": "ok", "records": records("entry", [row("entry", name="l.txt"), row("entry", name="m.txt")])})
    c.add("filter-extension", "Keeps entries by extension.", fs, 'find "." |> filter extension == "md"',
          report={"status": "ok", "records": records("entry", [row("entry", path="docs/g.md"), row("entry", path="src/d/f.md")])})
    c.add("first-two", "Keeps the first two entries.", fs, 'find "." glob: "**/*.go" |> first 2',
          report={"status": "ok", "records": records("entry", found(layout, "**/*.go")[:2])})
    c.add("count", "Counts the entries.", fs, 'find "." glob: "**/*.go" |> count', report={"status": "ok"})
    c.add("fuzzy", "Ranks entries by similarity of the name.", fs, 'find "." fuzzy: "egm"', report={"status": "ok"})
    c.add("missing-directory", "Finding under a missing directory fails.", fs, 'find "absent"',
          report={"status": "failed", "error": {"code": "E_NOT_FOUND"}})
    c.add("follow-links-off", "Does not descend into linked directories by default.",
          {"entries": [entry("real/x.txt", "x\n"), entry("link", target="real")]}, 'find "." glob: "**/x.txt"',
          report={"status": "ok", "records": records("entry", [row("entry", path="real/x.txt")])}, requires=("os:unix",))
    c.add("follow-links-on", "Descends into linked directories when asked.",
          {"entries": [entry("real/x.txt", "x\n"), entry("link", target="real")]}, 'find "." glob: "**/x.txt" follow_links: true',
          report={"status": "ok", "records": records("entry", [row("entry", path="link/x.txt"), row("entry", path="real/x.txt")])},
          requires=("os:unix",))
    c.add("list-top-level", "Lists one directory level, hidden entries included.", fs, 'list "."',
          report={"status": "ok", "kind": "list", "records": records("entry", matched=6)})
    c.add("list-without-hidden", "Lists without hidden entries.", fs, 'list "." hidden: false',
          report={"status": "ok", "records": records("entry", matched=5)})
    c.add("list-files", "Lists only files.", fs, 'list "." type: file',
          report={"status": "ok", "records": records("entry", [row("entry", name=".env"), row("entry", name="a.go"),
                                                             row("entry", name="b.txt")])})
    c.add("list-depth-two", "Lists two directory levels.", fs, 'list "src" depth: 2',
          report={"status": "ok", "records": records("entry", matched=4)})
    c.add("list-sort-name-desc", "Lists in descending name order.", tree({"a": "", "b": "", "c": ""}), 'list "." order: desc',
          report={"status": "ok", "records": records("entry", [row("entry", name="c"), row("entry", name="b"), row("entry", name="a")])})
    c.add("list-tree", "Lists a tree view.", fs, 'list "." depth: 3 view: tree', report={"status": "ok"})
    c.add("list-sizes", "Lists directory size totals.", fs, 'list "." view: sizes', report={"status": "ok"})
    c.add("list-missing", "Listing a missing directory fails.", fs, 'list "absent"',
          report={"status": "failed", "error": {"code": "E_NOT_FOUND"}})
    c.add("list-empty-directory", "Lists an empty directory.", fs, 'list "empty"',
          report={"status": "ok", "records": records("entry", [])})
    c.add("info-file", "Describes a file.", fs, 'info "a.go"',
          report={"status": "ok", "kind": "info", "records": records("entry", [row("entry", path="a.go", name="a.go", type="file",
                                                                                 exists=True, size=10, extension="go")])})
    c.add("info-directory", "Describes a directory.", fs, 'info "src"',
          report={"status": "ok", "records": records("entry", [row("entry", path="src", type="directory", exists=True)])})
    c.add("info-missing", "Describes a path that does not exist.", fs, 'info "absent.txt"',
          report={"status": "ok", "records": records("entry", [row("entry", path="absent.txt", exists=False, size=0)])})
    c.add("info-several", "Describes several paths.", fs, 'info ["a.go", "b.txt", "absent"]',
          report={"status": "ok", "records": records("entry", matched=3)})
    hello = "hello\n"
    c.add("info-sha256", "Computes the SHA-256 hash of a file.", tree({"h.txt": hello}), 'info "h.txt" hash: sha256',
          report={"status": "ok", "records": records("entry", [row(
              "entry", hash="sha256:5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03")])})
    c.add("info-text-details", "Describes the text of a file.", tree({"w.txt": "a\r\nb\r\n"}), 'info "w.txt" details: [text]',
          report={"status": "ok", "records": records("entry", [row("entry", line_ending="crlf", lines=2)])})
    c.add("info-mode", "Reports the permission bits.", {"entries": [entry("run.sh", "#!/bin/sh\n", mode="0755")]}, 'info "run.sh"',
          report={"status": "ok", "records": records("entry", [row("entry", mode="0755")])}, requires=("os:unix",))
    c.add("info-link", "Describes a symbolic link with its target.",
          {"entries": [entry("t.txt", "t\n"), entry("l.txt", target="t.txt")]}, 'info "l.txt"',
          report={"status": "ok", "records": records("entry", [row("entry", type="link")])}, requires=("os:unix",))
    c.add("hash-command", "Hashes a file with hash.", tree({"h.txt": hello}), 'hash "h.txt"',
          report={"status": "ok", "records": records("entry", [row(
              "entry", hash="sha256:5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03")])})
    c.add("hash-md5", "Hashes a file with MD5.", tree({"h.txt": hello}), 'hash "h.txt" algorithm: md5',
          report={"status": "ok", "records": records("entry", [row("entry", hash="md5:b1946ac92492d2347c6235b4d2611184")])})
    c.add("select-fields", "Keeps selected fields of the found entries.", fs, 'find "." glob: "*.go" depth: 1 |> select name size',
          report={"status": "ok", "records": records(
              "entry", [{"kind": "entry", "fields": {"name": {"t": "text", "v": "a.go"}, "size": {"t": "size", "v": 10}}}])})
    c.add("summarize-size", "Sums the sizes of the found files.", sized, 'find "." |> summarize size', report={"status": "ok"})
    c.add("group-extension", "Groups the found files by extension.", fs, 'find "." |> group extension', report={"status": "ok"})
    return c


def write_scenarios() -> Collection:
    """Writing files: creation, overwrite, append, encodings and validation."""
    c = Collection("files", "write")
    base = {"keep.txt": "keep\n"}
    fs = tree(base)
    c.add("create", "Creates a new file with the text.", fs, 'write "new.txt" "hello\\n" mode: create',
          report={"status": "ok", "kind": "write", "summary": {"files_created": 1},
                  "changes": [{"path": "${base}/new.txt", "action": "create"}]},
          after=changed(base, new_dot_txt="hello\n"))
    c.add("create-existing", "Creating an existing file fails and changes nothing.", fs, 'write "keep.txt" "x" mode: create',
          report={"status": "failed", "error": {"code": "E_EXISTS"}})
    c.add("create-parents", "Creates missing parent directories.", fs, 'write "a/b/c.txt" "x\\n" mode: create',
          report={"status": "ok"}, after=changed(base, a__b__c_dot_txt="x\n"))
    c.add("create-without-parents", "Fails without parents when parents is false.", fs,
          'write "a/b/c.txt" "x\\n" mode: create parents: false',
          report={"status": "failed", "error": {"code": "E_NOT_FOUND"}})
    c.add("overwrite", "Replaces the content of a file.", fs, 'write "keep.txt" "new\\n" mode: overwrite',
          report={"status": "ok", "summary": {"files_modified": 1},
                  "notable": [{"code": "overwrote_existing"}]}, after=changed(base, keep_dot_txt="new\n"))
    c.add("overwrite-missing", "Overwrite creates a missing file.", fs, 'write "n.txt" "n\\n" mode: overwrite',
          report={"status": "ok"}, after=changed(base, n_dot_txt="n\n"))
    c.add("append", "Appends to a file.", fs, 'write "keep.txt" "more\\n" mode: append',
          report={"status": "ok"}, after=changed(base, keep_dot_txt="keep\nmore\n"))
    c.add("prepend", "Prepends to a file.", fs, 'write "keep.txt" "first\\n" mode: prepend',
          report={"status": "ok"}, after=changed(base, keep_dot_txt="first\nkeep\n"))
    c.add("ensure-newline", "Adds a final line ending when asked.", fs, 'write "e.txt" "x" mode: create ensure_newline: true',
          report={"status": "ok"}, after=changed(base, e_dot_txt="x\n"))
    c.add("no-newline-added", "Writes the text as given by default.", fs, 'write "e.txt" "x" mode: create',
          report={"status": "ok"}, after=changed(base, e_dot_txt="x"))
    c.add("empty-text", "Writes an empty file.", fs, 'write "e.txt" "" mode: create', report={"status": "ok"},
          after=changed(base, e_dot_txt=""))
    c.add("emptied-is-notable", "Emptying a file is an unusual effect.", fs, 'write "keep.txt" "" mode: overwrite',
          report={"status": "ok", "notable": [{"code": "file_emptied", "severity": "warning", "path": "${base}/keep.txt"},
                                              {"code": "overwrote_existing"}]},
          after=changed(base, keep_dot_txt=""))
    c.add("identical-content", "Writing identical content is reported as unchanged.", fs, 'write "keep.txt" "keep\\n" mode: overwrite',
          report={"status": "ok", "notable": [{"code": "unchanged"}]})
    c.add("crlf-line-endings", "Writes CRLF line endings when asked.", fs,
          'write "w.txt" "a\\nb\\n" mode: create line_ending: crlf', report={"status": "ok"},
          after=changed(base, w_dot_txt="a\r\nb\r\n"))
    crlf = {"w.txt": "a\r\nb\r\n"}
    c.add("keeps-crlf-on-append", "Appending to a CRLF file keeps its line endings.", tree(crlf),
          'write "w.txt" "c\\n" mode: append', report={"status": "ok"}, after=tree({"w.txt": "a\r\nb\r\nc\r\n"}))
    legacy = {"l.txt": "Žilina\n".encode("cp1250")}
    c.add("keeps-legacy-encoding", "Overwriting a Windows-1250 file keeps its encoding.", tree(legacy),
          'write "l.txt" "Šala\\n" mode: overwrite',
          report={"status": "ok", "notable": [{"code": "encoding_preserved"}, {"code": "overwrote_existing"}]},
          after=tree({"l.txt": "Šala\n".encode("cp1250")}))
    c.add("explicit-encoding", "Writes a new file in the given encoding.", fs,
          'write "n.txt" "Šala\\n" mode: create encoding: "windows-1250"', report={"status": "ok"},
          after=changed(base, n_dot_txt="Šala\n".encode("cp1250")))
    c.add("utf16-with-bom", "Writes a UTF-16 file with a byte order mark.", fs,
          'write "u.txt" "hi\\n" mode: create encoding: "utf-16le" bom: true', report={"status": "ok"},
          after=changed(base, u_dot_txt=b"\xff\xfe" + "hi\n".encode("utf-16-le")))
    c.add("utf8-bom", "Writes a UTF-8 byte order mark when asked.", fs, 'write "b.txt" "x\\n" mode: create bom: true',
          report={"status": "ok"}, after=changed(base, b_dot_txt=b"\xef\xbb\xbfx\n"))
    c.add("multiline-raw-text", "Writes a raw multi-line text.", fs,
          'write "Dockerfile" r#"FROM alpine:3.20\nCMD ["sh"]\n"# mode: create', report={"status": "ok"},
          after=changed(base, Dockerfile='FROM alpine:3.20\nCMD ["sh"]\n'))
    c.add("from-pipeline", "Writes the lines of a pipeline to a file.", tree({"in.txt": "b\na\nc\n"}),
          'write "out.txt" from: (lines "in.txt" |> sort text) mode: create format: text',
          report={"status": "ok"}, after=tree({"in.txt": "b\na\nc\n", "out.txt": "a\nb\nc\n"}))
    c.add("sorted-into-same-file", "Sorts the lines of a file into the same file.", tree({"d.txt": "b\na\nc\n"}),
          'lines "d.txt" |> sort text |> write "d.txt" mode: overwrite', report={"status": "ok"},
          after=tree({"d.txt": "a\nb\nc\n"}))
    c.add("json-valid", "Writes a valid JSON document.", fs, 'write "c.json" "{\\"a\\": 1}\\n" mode: create',
          report={"status": "ok"}, after=changed(base, c_dot_json='{"a": 1}\n'))
    c.add("json-invalid-rejected", "Writing an invalid JSON document is rejected by validation.", fs,
          'write "c.json" "{\\"a\\": " mode: create', report={"status": "failed", "error": {"code": "E_DOC_PARSE"}})
    c.add("json-invalid-allowed", "Validation off writes the text as given.", fs,
          'write "c.json" "{\\"a\\": " mode: create validate: off', report={"status": "ok"},
          after=changed(base, c_dot_json='{"a": '))
    c.add("batch", "Writes several files in one operation.", fs,
          'write files: [{path: "x.txt", text: "x\\n"}, {path: "y.txt", text: "y\\n"}] mode: create',
          report={"status": "ok", "summary": {"files_created": 2}},
          after=changed(base, x_dot_txt="x\n", y_dot_txt="y\n"))
    c.add("if-match-stale", "A write with an outdated entity tag fails and changes nothing.", fs,
          steps=[step('read "keep.txt"', report={"status": "ok"}),
                 step('write "keep.txt" "changed\\n" mode: overwrite', report={"status": "ok"}),
                 step('write "keep.txt" "late\\n" mode: overwrite if_match: "${step1/report/records/rows/0/fields/etag/v}"',
                      report={"status": "failed", "error": {"code": "E_STALE"}},
                      after=changed(base, keep_dot_txt="changed\n"))])
    c.add("file-mode", "Creates a file with the given permission bits.", fs,
          'write "run.sh" "#!/bin/sh\\n" mode: create file_mode: "0755"', report={"status": "ok"},
          after={"entries": [entry("keep.txt", "keep\n"), entry("run.sh", "#!/bin/sh\n", mode="0755")]},
          requires=("os:unix",))
    c.add("preview", "A preview of a write changes nothing.", fs, 'write "new.txt" "x\\n" mode: create',
          options={"preview": True}, report={"status": "planned", "kind": "write"})
    c.add("bytes", "Writes binary data.", fs, 'write "b.bin" data: b64"AAEC" mode: create', report={"status": "ok"},
          after=changed(base, b_dot_bin=b"\x00\x01\x02"))
    return c


def edit_scenarios() -> Collection:
    """Replacing, inserting and editing text inside files."""
    c = Collection("files", "edit")
    go = {"a.go": "package a\n\nfunc oldName() {}\n\nfunc b() { oldName() }\n"}
    fs = tree(go)
    rep = go["a.go"].replace("oldName", "newName")
    c.add("replace-all", "Replaces every occurrence in a file.", fs, 'replace "oldName" "newName" in: ["a.go"]',
          report={"status": "ok", "kind": "replace", "summary": {"files_modified": 1, "replacements": 2}},
          after=tree({"a.go": rep}))
    c.add("replace-first", "Replaces the first occurrence only.", fs, 'replace "oldName" "newName" in: ["a.go"] occurrence: 1',
          report={"status": "ok", "summary": {"replacements": 1}},
          after=tree({"a.go": go["a.go"].replace("oldName", "newName", 1)}))
    c.add("replace-second", "Replaces the second occurrence only.", fs, 'replace "oldName" "newName" in: ["a.go"] occurrence: 2',
          report={"status": "ok"}, after=tree({"a.go": "package a\n\nfunc oldName() {}\n\nfunc b() { newName() }\n"}))
    c.add("replace-regex-group", "Replaces with a capture group.", tree({"v.txt": "version=1.2.3\n"}),
          r'replace /version=(\d+)\.(\d+)\.(\d+)/ "version=$1.$2.4" in: ["v.txt"]', report={"status": "ok"},
          after=tree({"v.txt": "version=1.2.4\n"}))
    c.add("replace-literal-dollar", "A replacement without groups keeps a dollar sign when templates are off.",
          tree({"p.txt": "price\n"}), 'replace "price" "$5" in: ["p.txt"] template: false', report={"status": "ok"},
          after=tree({"p.txt": "$5\n"}))
    c.add("replace-expect-holds", "A replacement with the expected count applies.", fs,
          'replace "oldName" "newName" in: ["a.go"] expect: 2', report={"status": "ok"}, after=tree({"a.go": rep}))
    c.add("replace-expect-fails", "A replacement with another count than expected changes nothing.", fs,
          'replace "oldName" "newName" in: ["a.go"] expect: 3', report={"status": "failed"})
    c.add("replace-no-match-skip", "A replacement without a match succeeds and changes nothing by default.", fs,
          'replace "absent" "x" in: ["a.go"]', report={"status": "ok", "notable": [{"code": "nothing_matched"}]})
    c.add("replace-no-match-error", "A replacement without a match fails when asked.", fs,
          'replace "absent" "x" in: ["a.go"] on_missing: error', report={"status": "failed", "error": {"code": "E_NO_MATCH"}})
    c.add("replace-whole-word", "Replaces whole words only.", tree({"w.txt": "cat catalog cat\n"}),
          'replace "cat" "dog" in: ["w.txt"] whole_word: true', report={"status": "ok"}, after=tree({"w.txt": "dog catalog dog\n"}))
    c.add("replace-case-insensitive", "Replaces without letter case when asked.", tree({"w.txt": "Foo foo FOO\n"}),
          'replace "foo" "bar" in: ["w.txt"] case_sensitive: false', report={"status": "ok"}, after=tree({"w.txt": "bar bar bar\n"}))
    multi = {"a.txt": "x = 1\n", "b.txt": "x = 2\n", "c.md": "x = 3\n"}
    c.add("replace-in-directory", "Replaces in every file of a directory matching a glob.", tree(multi),
          'replace "x =" "y =" in: ["."] glob: "*.txt"', report={"status": "ok", "summary": {"files_modified": 2}},
          after=tree({"a.txt": "y = 1\n", "b.txt": "y = 2\n", "c.md": "x = 3\n"}))
    c.add("replace-found-matches", "Replaces the matches found by a search.", fs, 'search "oldName" "." |> replace "newName"',
          report={"status": "ok", "summary": {"replacements": 2}}, after=tree({"a.go": rep}))
    c.add("replace-assert-before", "An assertion on the number of files before the change holds and the change applies.", fs,
          'search "oldName" "." |> assert paths: 1 |> replace "newName"', report={"status": "ok"}, after=tree({"a.go": rep}))
    c.add("replace-assert-fails", "A failed assertion before the change changes nothing.", fs,
          'search "oldName" "." |> assert paths: 2 |> replace "newName"',
          report={"status": "failed", "error": {"code": "E_ASSERT", "stage": "3"}})
    c.add("replace-keeps-crlf", "A replacement keeps CRLF line endings.", tree({"w.txt": "old\r\nkeep\r\n"}),
          'replace "old" "new" in: ["w.txt"]', report={"status": "ok"}, after=tree({"w.txt": "new\r\nkeep\r\n"}))
    c.add("replace-keeps-cp1250", "A replacement keeps the Windows-1250 encoding.",
          tree({"l.txt": "Brno Žilina\n".encode("cp1250")}), 'replace "Brno" "Košice" in: ["l.txt"]',
          report={"status": "ok", "notable": [{"code": "encoding_preserved"}]},
          after=tree({"l.txt": "Košice Žilina\n".encode("cp1250")}))
    c.add("replace-keeps-utf16", "A replacement keeps the UTF-16 encoding and byte order mark.",
          tree({"u.txt": b"\xff\xfe" + "alpha\n".encode("utf-16-le")}), 'replace "alpha" "beta" in: ["u.txt"]',
          report={"status": "ok"}, after=tree({"u.txt": b"\xff\xfe" + "beta\n".encode("utf-16-le")}))
    c.add("replace-unrepresentable", "A character the encoding of the file cannot hold fails and changes nothing.",
          tree({"l.txt": "a\n".encode("cp1250")}), 'replace "a" "日本" in: ["l.txt"]',
          report={"status": "failed", "error": {"code": "E_ENCODING"}})
    c.add("replace-binary-skipped", "Binary files are skipped by a replacement in a directory.",
          tree({"t.txt": "x\n", "b.bin": b"\x00x\x00"}), 'replace "x" "y" in: ["."]',
          report={"status": "ok", "notable": [{"code": "binary_skipped", "count": 1}]},
          after=tree({"t.txt": "y\n", "b.bin": b"\x00x\x00"}))
    c.add("replace-breaks-json", "A replacement that breaks a JSON document is rejected.", tree({"c.json": '{"a": 1}\n'}),
          'replace "1}" "1" in: ["c.json"]', report={"status": "failed", "error": {"code": "E_DOC_PARSE"}})
    c.add("replace-whitespace-normalize", "Matches across different runs of white space by default.",
          tree({"s.txt": "if  (x)   {\n"}), 'replace "if (x) {" "if (y) {" in: ["s.txt"]', report={"status": "ok"},
          after=tree({"s.txt": "if (y) {\n"}))
    c.add("replace-whitespace-exact", "Exact white space does not match different spacing.",
          tree({"s.txt": "if  (x)\n"}), 'replace "if (x)" "if (y)" in: ["s.txt"] whitespace: exact',
          report={"status": "ok", "notable": [{"code": "nothing_matched"}]})
    c.add("replace-multiline", "Replaces a match that spans lines.", tree({"m.txt": "a\nb\nc\n"}),
          r'replace /a\nb/ "ab" in: ["m.txt"] multiline: true', report={"status": "ok"}, after=tree({"m.txt": "ab\nc\n"}))
    c.add("replace-preview", "A preview of a replacement shows the difference and changes nothing.", fs,
          'replace "oldName" "newName" in: ["a.go"]', options={"preview": True},
          report={"status": "planned", "kind": "replace", "summary": {"replacements": 2}})
    c.add("replace-lines", "Replaces in the lines produced by lines.", tree({"c.ini": "debug = true\nport = 80\n"}),
          'lines "c.ini" |> filter text ~ /^debug/ |> replace "true" "false"', report={"status": "ok"},
          after=tree({"c.ini": "debug = false\nport = 80\n"}))
    lines = {"main.go": "package main\n\nfunc main() {}\n"}
    c.add("insert-after-line", "Inserts a line after the matching line.", tree(lines),
          'lines "main.go" |> filter text == "package main" |> insert r#"import "fmt""# position: after',
          report={"status": "ok"}, after=tree({"main.go": 'package main\nimport "fmt"\n\nfunc main() {}\n'}))
    c.add("insert-before-line", "Inserts a line before a line number.", tree(lines), 'insert "// header" in: "main.go" line: 1 position: before',
          report={"status": "ok"}, after=tree({"main.go": "// header\npackage main\n\nfunc main() {}\n"}))
    c.add("insert-at-start", "Inserts at the start of a file.", tree(lines), 'insert "// top\\n" in: "main.go" position: start',
          report={"status": "ok"}, after=tree({"main.go": "// top\npackage main\n\nfunc main() {}\n"}))
    c.add("insert-at-end", "Inserts at the end of a file.", tree(lines), 'insert "// end\\n" in: "main.go" position: end',
          report={"status": "ok"}, after=tree({"main.go": "package main\n\nfunc main() {}\n// end\n"}))
    c.add("insert-after-line-number", "Inserts after a line number by default.", tree(lines), 'insert "// two" in: "main.go" line: 2',
          report={"status": "ok"}, after=tree({"main.go": "package main\n\n// two\nfunc main() {}\n"}))
    c.add("insert-crlf", "An inserted line takes the CRLF line ending of the file.", tree({"w.txt": "a\r\nb\r\n"}),
          'insert "x" in: "w.txt" line: 1', report={"status": "ok"}, after=tree({"w.txt": "a\r\nx\r\nb\r\n"}))
    edit_src = {"e.go": "line1\nline2 oldName(\nline3\nline4\nline5\n"}
    c.add("edit-all-changes", "Applies a list of changes to one file in one operation.", tree(edit_src),
          'edit "e.go" changes: [{replace: "oldName(", with: "newName("}, {delete_lines: 4..5}]',
          report={"status": "ok", "kind": "edit"}, after=tree({"e.go": "line1\nline2 newName(\nline3\n"}))
    c.add("edit-insert-and-replace", "Inserts and replaces in one operation; line numbers refer to the original file.",
          tree(edit_src), 'edit "e.go" changes: [{insert: "// top", line: 1, position: before}, {replace: "line3", with: "LINE3"}]',
          report={"status": "ok"}, after=tree({"e.go": "// top\nline1\nline2 oldName(\nLINE3\nline4\nline5\n"}))
    c.add("edit-all-or-nothing", "A change without a match fails the whole edit when asked; nothing changes.", tree(edit_src),
          'edit "e.go" changes: [{replace: "oldName(", with: "newName("}, {replace: "absent", with: "x"}] on_missing: error',
          report={"status": "failed", "error": {"code": "E_NO_MATCH"}})
    c.add("edit-expect-count", "An edit with the expected number of changes applies.", tree(edit_src),
          'edit "e.go" changes: [{replace: "line1", with: "L1"}, {replace: "line5", with: "L5"}] expect: 2',
          report={"status": "ok"}, after=tree({"e.go": "L1\nline2 oldName(\nline3\nline4\nL5\n"}))
    c.add("edit-preview", "A preview of an edit changes nothing.", tree(edit_src), 'edit "e.go" changes: [{replace: "line1", with: "L1"}]',
          options={"preview": True}, report={"status": "planned"})
    c.add("set-json-key", "Sets a key of a JSON document and keeps its layout.",
          tree({"package.json": '{\n  "name": "app",\n  "scripts": {\n    "test": "jest"\n  }\n}\n'}),
          'set "package.json" key: "scripts.test" value: "vitest"', report={"status": "ok", "kind": "set"},
          after=tree({"package.json": '{\n  "name": "app",\n  "scripts": {\n    "test": "vitest"\n  }\n}\n'}))
    c.add("set-toml-keeps-comments", "Sets a key of a TOML document and keeps comments.",
          tree({"Cargo.toml": '[package]\n# release name\nversion = "0.3.0"\n'}),
          'set "Cargo.toml" key: "package.version" value: "0.4.0"', report={"status": "ok"},
          after=tree({"Cargo.toml": '[package]\n# release name\nversion = "0.4.0"\n'}))
    c.add("set-creates-key", "Creates a missing key with its parents.", tree({"c.json": '{"a": 1}\n'}),
          'set "c.json" key: "b.c" value: 2', report={"status": "ok", "notable": [{"code": "keys_created"}]})
    c.add("set-without-create", "Setting a missing key without creation fails.", tree({"c.json": '{"a": 1}\n'}),
          'set "c.json" key: "b" value: 2 create: false', report={"status": "failed", "error": {"code": "E_KEY_NOT_FOUND"}})
    c.add("remove-line", "Removes the lines matching a condition.", tree({"c.ini": "a = 1\ndebug = true\nb = 2\n"}),
          r'lines "c.ini" |> filter text ~ /^\s*debug\s*=/ |> remove', report={"status": "ok"},
          after=tree({"c.ini": "a = 1\nb = 2\n"}))
    c.add("convert-line-endings", "Converts CRLF to LF.", tree({"w.txt": "a\r\nb\r\n"}), 'convert "w.txt" line_ending: lf',
          report={"status": "ok", "kind": "convert"}, after=tree({"w.txt": "a\nb\n"}))
    c.add("convert-encoding", "Converts Windows-1250 to UTF-8.", tree({"l.txt": "Žilina\n".encode("cp1250")}),
          'convert "l.txt" encoding: "utf-8"', report={"status": "ok"}, after=tree({"l.txt": "Žilina\n"}))
    c.add("patch", "Applies a unified difference.", tree({"a.txt": "one\ntwo\n"}),
          'patch "--- a/a.txt\\n+++ b/a.txt\\n@@ -1,2 +1,2 @@\\n one\\n-two\\n+TWO\\n" root: "."',
          report={"status": "ok", "kind": "patch"}, after=tree({"a.txt": "one\nTWO\n"}))
    c.add("patch-rejected", "A hunk that does not apply changes nothing.", tree({"a.txt": "one\nthree\n"}),
          'patch "--- a/a.txt\\n+++ b/a.txt\\n@@ -1,2 +1,2 @@\\n one\\n-two\\n+TWO\\n" root: "."',
          report={"status": "failed", "error": {"code": "E_PATCH_REJECTED"}})
    return c


def remove_scenarios() -> Collection:
    """Removing, moving, copying and creating entries."""
    c = Collection("files", "entries")
    base = {"a.txt": "a\n", "dir/b.txt": "b\n", "dir/sub/c.txt": "c\n", "keep.txt": "k\n"}
    fs = tree(base)
    c.add("remove-file", "Removes a file.", fs, 'remove "a.txt"',
          report={"status": "ok", "kind": "remove", "summary": {"files_deleted": 1},
                  "changes": [{"path": "${base}/a.txt", "action": "delete"}]}, after=changed(base, a_dot_txt=REMOVED))
    c.add("remove-tree", "Removes a directory tree as one target.", fs, 'remove "dir"',
          report={"status": "ok", "notable": [{"code": "delete_tree"}]},
          after=tree({"a.txt": "a\n", "keep.txt": "k\n"}))
    c.add("remove-several", "Removes several paths in one operation.", fs, 'remove ["a.txt", "keep.txt"]',
          report={"status": "ok", "summary": {"files_deleted": 2}}, after=tree({"dir/b.txt": "b\n", "dir/sub/c.txt": "c\n"}))
    c.add("remove-missing-skip", "Removing a missing path succeeds and changes nothing by default.", fs, 'remove "absent.txt"',
          report={"status": "ok"})
    c.add("remove-missing-error", "Removing a missing path fails when asked.", fs, 'remove "absent.txt" on_missing: error',
          report={"status": "failed", "error": {"code": "E_NOT_FOUND"}})
    c.add("remove-found", "Removes the files found by find.", fs, 'find "dir" glob: "**/*.txt" |> remove',
          report={"status": "ok", "summary": {"files_deleted": 2}}, after=tree({"a.txt": "a\n", "keep.txt": "k\n", "dir/sub": None}))
    c.add("remove-path-with-space", "Removes a path with a space.", tree({"my file.txt": "x\n"}), 'remove "my file.txt"',
          report={"status": "ok"}, after=tree({}))
    c.add("remove-preview", "A preview of a removal lists the targets and changes nothing.", fs, 'remove "dir"',
          options={"preview": True}, report={"status": "planned", "kind": "remove"})
    c.add("move-file", "Moves a file to a new path.", fs, 'move "a.txt" to: "moved.txt"',
          report={"status": "ok", "kind": "move"}, after=changed(base, a_dot_txt=REMOVED, moved_dot_txt="a\n"))
    c.add("move-into", "Moves a file into a directory.", fs, 'move "a.txt" into: "dir"',
          report={"status": "ok"}, after=changed(base, a_dot_txt=REMOVED, dir__a_dot_txt="a\n"))
    c.add("move-conflict-error", "Moving onto an existing file fails when asked.", fs,
          'move "a.txt" to: "keep.txt" on_conflict: error', report={"status": "failed", "error": {"code": "E_EXISTS"}})
    c.add("move-conflict-overwrite", "Moving onto an existing file replaces it by default.", fs, 'move "a.txt" to: "keep.txt"',
          report={"status": "ok", "notable": [{"code": "overwrote_existing"}]},
          after=tree({"keep.txt": "a\n", "dir/b.txt": "b\n", "dir/sub/c.txt": "c\n"}))
    c.add("move-found", "Moves the files found by find into a directory.", tree({"in/a.log": "a\n", "in/b.log": "b\n", "in/c.txt": "c\n", "arch": None}),
          'find "in" glob: "*.log" depth: 1 |> move into: "arch"', report={"status": "ok"},
          after=tree({"in/c.txt": "c\n", "arch/a.log": "a\n", "arch/b.log": "b\n"}))
    c.add("copy-file", "Copies a file.", fs, 'copy "a.txt" to: "copy.txt"', report={"status": "ok", "kind": "copy"},
          after=changed(base, copy_dot_txt="a\n"))
    c.add("copy-tree", "Copies a directory tree.", fs, 'copy "dir" to: "dir2"', report={"status": "ok"},
          after=changed(base, dir2__b_dot_txt="b\n", dir2__sub__c_dot_txt="c\n"))
    c.add("copy-conflict-skip", "Copying onto an existing file leaves it when asked.", fs,
          'copy "a.txt" to: "keep.txt" on_conflict: skip', report={"status": "ok"})
    c.add("rename-file", "Renames a file in its directory.", fs, 'rename path: "a.txt" to: "z.txt"',
          report={"status": "ok", "kind": "rename"}, after=changed(base, a_dot_txt=REMOVED, z_dot_txt="a\n"))
    c.add("rename-pattern", "Renames found entries by a pattern.", tree({"x.log": "1", "y.log": "2"}),
          r'find "." glob: "*.log" depth: 1 |> rename /\.log$/ ".txt"', report={"status": "ok"},
          after=tree({"x.txt": "1", "y.txt": "2"}))
    c.add("create-directory", "Creates a directory with its parents.", fs, 'create "out/logs" type: directory',
          report={"status": "ok", "kind": "create"}, after=changed(base, out__logs=None))
    c.add("create-file-exists-touch", "Creating an existing file with touch keeps its content.", fs,
          'create "a.txt" type: file on_exists: touch', report={"status": "ok"})
    c.add("create-file-exists-error", "Creating an existing file fails when asked.", fs,
          'create "a.txt" type: file on_exists: error', report={"status": "failed", "error": {"code": "E_EXISTS"}})
    c.add("create-link", "Creates a symbolic link.", fs, 'create "l" type: link target: "dir"', report={"status": "ok"},
          after={"entries": [entry(p, v) for p, v in base.items()] + [entry("l", target="dir")]}, requires=("os:unix",))
    return c


def notebook_scenarios() -> Collection:
    """Cells of notebooks."""
    c = Collection("files", "notebook")
    nb = {
        "cells": [
            {"cell_type": "markdown", "id": "m1", "metadata": {}, "source": ["# Title"]},
            {"cell_type": "code", "id": "c1", "metadata": {}, "execution_count": 1, "source": ["print(1)"],
             "outputs": [{"output_type": "stream", "name": "stdout", "text": ["1\n"]}]},
        ],
        "metadata": {}, "nbformat": 4, "nbformat_minor": 5,
    }
    text = json.dumps(nb, indent=1) + "\n"
    fs = tree({"a.ipynb": text})
    c.add("cells-outline", "Lists the cells of a notebook.", fs, 'notebook.cells "a.ipynb"',
          report={"status": "ok", "records": records("cell", matched=2)})
    c.add("cells-code-only", "Lists the code cells.", fs, 'notebook.cells "a.ipynb" cell_type: code',
          report={"status": "ok", "records": records("cell", matched=1)})
    c.add("read-cell", "Reads one cell of a notebook.", fs, 'read "a.ipynb" cell: 1', report={"status": "ok"})
    c.add("insert-cell", "Inserts a cell.", fs, 'notebook.insert "a.ipynb" index: 2 cell_type: code source: "print(2)"',
          report={"status": "ok", "kind": "notebook.insert"})
    c.add("replace-cell", "Replaces the source of a cell.", fs, 'notebook.replace "a.ipynb" id: "c1" source: "print(3)"',
          report={"status": "ok"})
    c.add("remove-cell", "Removes a cell.", fs, 'notebook.remove "a.ipynb" id: "m1"', report={"status": "ok"})
    c.add("clear-outputs", "Clears the outputs of all cells.", fs, 'notebook.clear_outputs "a.ipynb"', report={"status": "ok"})
    c.add("move-cell", "Moves a cell.", fs, 'notebook.move "a.ipynb" from: 1 to: 0', report={"status": "ok"})
    c.add("export-script", "Exports the code cells to a script.", fs, 'notebook.export "a.ipynb" to: "a.py" format: script',
          report={"status": "ok"}, after={"exhaustive": False, "entries": [entry("a.py", "print(1)\n")]})
    c.add("invalid-notebook", "A damaged notebook is reported as a format error.", tree({"b.ipynb": "{"}),
          'notebook.cells "b.ipynb"', report={"status": "failed"})
    return c


def collections() -> list[Collection]:
    """Returns every parity collection of the file tools."""
    return [read_scenarios(), search_scenarios(), find_scenarios(), write_scenarios(), edit_scenarios(),
            remove_scenarios(), notebook_scenarios()]
