# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Journal cases: undo of every row of the coverage table, and the semantics of undo and redo.

Every row of `tables/journal-coverage.toml` outside the group `git` has a case
`journal/coverage/<row>`. A row that keeps the previous state performs the
operation, undoes it and compares the whole tree with the state before the
operation, byte for byte and with the metadata the snapshot lists. A row
without preservation checks the irreversible reasons of the report instead.
The cases of `journal/semantics` cover the selectors of undo, redo, the
conflict choices, the history, the absence of expiry information in results
and expiry by the retention limits.

Every case belongs to the class `optional:journal`, which is mandatory for
implementations that declare undo; the expiry by age additionally needs the
test clock of the feature `testkit`.
"""

from __future__ import annotations

from parity_model import (
    REMOVED, UNCHANGED, Collection, changed, entry, gzip_bytes, records, row, tree, zip_bytes,
)

JOURNAL = "journal"
NO_PREIMAGE = "../exec/config/no-preimage.toml"
JOURNAL_DISABLED = "../exec/config/journal-disabled.toml"
BELOW_RESERVE = "../exec/config/below-reserve.toml"
OP_SIZE_LIMIT = "../exec/config/op-size-limit.toml"
SIZE_BUDGET = "../exec/config/size-budget.toml"
SIZE_BUDGET_KEEP = "../exec/config/size-budget-keep.toml"
VOLUME_MAX_SIZE = "../exec/config/volume-max-size.toml"
BIG = "x" * 4095 + "\n"
"""File content above the size limits of 1 KiB set by the configurations of the size cases."""
HALF_BUDGET = "y" * 699 + "\n"
"""File content of which one copy fits a size limit of 1 KiB and two copies do not."""
FIXTURE = "nurt-conformance-fixture"
REGISTRY = "registry:/HKCU/Software/NurtConformance"
OLD_TIME = "2020-01-01T00:00:00Z"
NEW_TIME = "2026-01-01T00:00:00Z"

PENDING_ROWS = {
    "write-data": "the language has no literal of type bytes, so a program cannot state the bytes of the parameter data",
    "analyze.format-in-place": "the in-place mode is chosen by the formatter, not by the program",
}
"""Coverage rows without a case, with the precondition the suite cannot yet establish."""

INACTIVE_CASES = {
    "journal/semantics/journal-inactive": "disabled",
    "journal/semantics/journal-below-reserve": "below_reserve",
}
"""Cases whose configuration makes the journal inactive, with the reason of the inactive journal."""

TEXT = {"a.txt": "alpha\nbeta\n", "b.txt": "bravo\n"}
DIRS = {
    "a.txt": "alpha\nbeta\n",
    "e.txt": "echo\n",
    "dir/c.txt": "charlie\n",
    "dir/sub/d.txt": "delta\n",
}
DOCUMENT = {"c.json": '{\n  "name": "app",\n  "version": "1.0.0"\n}\n'}

NOTEBOOK_TAIL = '], "metadata": {}, "nbformat": 4, "nbformat_minor": 5}\n'
CODE_CELL = ('{"cell_type": "code", "id": "c1", "metadata": {}, "execution_count": null, '
             '"outputs": [], "source": ["print(1)"]}')
OUTPUT_CELL = ('{"cell_type": "code", "id": "c1", "metadata": {}, "execution_count": 1, '
               '"outputs": [{"output_type": "stream", "name": "stdout", "text": ["1\\n"]}], '
               '"source": ["print(1)"]}')
MARKDOWN_M = '{"cell_type": "markdown", "id": "m", "metadata": {}, "source": ["# A"]}'
MARKDOWN_N = '{"cell_type": "markdown", "id": "n", "metadata": {}, "source": ["# B"]}'
NOTEBOOK_CODE = {"a.ipynb": '{"cells": [' + CODE_CELL + NOTEBOOK_TAIL}
NOTEBOOK_OUTPUT = {"a.ipynb": '{"cells": [' + OUTPUT_CELL + NOTEBOOK_TAIL}
NOTEBOOK_ONE = {"a.ipynb": '{"cells": [' + MARKDOWN_M + NOTEBOOK_TAIL}
NOTEBOOK_TWO = {"a.ipynb": '{"cells": [' + MARKDOWN_M + ", " + MARKDOWN_N + NOTEBOOK_TAIL}
NOTEBOOK_OLD = {"a.ipynb": '{"cells": [{"cell_type": "markdown", "metadata": {}, "source": ["# A"]}], '
                           '"metadata": {}, "nbformat": 4, "nbformat_minor": 4}\n'}

PATCH_EDIT = 'patch "--- a/a.txt\\n+++ b/a.txt\\n@@ -1,2 +1,2 @@\\n one\\n-two\\n+TWO\\n" root: "${base}" strip: 1'
PATCH_FILES = ('patch "--- /dev/null\\n+++ b/n.txt\\n@@ -0,0 +1 @@\\n+new\\n'
               '--- a/b.txt\\n+++ /dev/null\\n@@ -1 +0,0 @@\\n-bravo\\n" root: "${base}" strip: 1')
SLEEPER = 'run ["sh", "-c", "sleep 30"] cwd: "${base}" background: true'


def op(n: int) -> str:
    """Returns the placeholder of the operation number reported by step `n`."""
    return f"${{step{n}/report/op}}"


def job(n: int) -> str:
    """Returns the placeholder of the job started by step `n`."""
    return f"${{step{n}/report/job/id}}"


def schedule(n: int) -> str:
    """Returns the placeholder of the identifier of the first schedule listed by step `n`."""
    return f"${{step{n}/report/records/rows/0/fields/id/v}}"


def ok(kind: str, **members: object) -> dict:
    """Returns an expected report of a successful command."""
    return {"status": "ok", "kind": kind, **members}


def failed(code: str) -> dict:
    """Returns an expected report of a failed call."""
    return {"status": "failed", "error": {"code": code}}


def irreversible(kind: str, *reasons: str) -> dict:
    """Returns an expected report of a successful command with irreversible effects."""
    return ok(kind, irreversible=[{"reason": r} for r in reasons])


UNDONE = ok("undo")

NO_JOURNAL_MEMBERS = [f"/report/{name}" for name in (
    "irreversible", "expires", "expires_at", "expiry", "undoable", "reversible", "journal")]
"""Members the report of a change or an undo never has: it states nothing about the journal."""
NO_EXPIRY_FIELDS = [f"/report/records/rows/*/fields/{name}" for name in (
    "expires", "expires_at", "expiry", "ttl", "remaining")]
"""Fields the records of `history` never have: the journal state is shown by `state` alone."""


def call(program: str | None = None, *, report: dict | None = None, after: object = None,
         method: str | None = None, params: dict | None = None, options: dict | None = None,
         expect_at: str | None = None, validate: str | None = None, expect: dict | None = None,
         absent: list[str] | None = None, second_after: object = None,
         setup_only: bool = False) -> dict:
    """Returns one step of a case; `setup_only` marks a step that states no expectation."""
    out: dict = {}
    if method is not None:
        out["method"] = method
    if params is not None:
        out["params"] = params
    if program is not None:
        out["program"] = program
    if options is not None:
        out["options"] = options
    if expect is not None:
        out["expect"] = expect
    elif report is not None:
        out["expect"] = {"report": report}
    if expect_at is not None:
        out["expect_at"] = expect_at
    if validate is not None:
        out["validate"] = validate
    if after is not None:
        out["fs_after"] = after
    if second_after is not None:
        out["second_volume_after"] = second_after
    if absent is not None:
        out["absent"] = absent
    if setup_only:
        out["setup_only"] = True
    return out


def undo(n: int, restored: dict) -> dict:
    """Returns a step that undoes the operation of step `n` and expects the tree `restored`."""
    return call(f"undo {op(n)}", report=UNDONE, after=restored)


def entry_rows(*exists: bool) -> dict:
    """Returns the expected records of `info` on paths that exist or not."""
    return records("entry", [row("entry", exists=e) for e in exists])


def operation(**fields: dict) -> dict:
    """Returns the expected operation record of `history` with some tagged fields."""
    return records("operation", [{"kind": "operation", "fields": fields}])


def enum(value: str) -> dict:
    """Returns a tagged enumeration value."""
    return {"t": "enum", "v": value}


def snapshot(*entries: dict) -> dict:
    """Returns a snapshot from explicit entries."""
    return {"entries": list(entries)}


def timed(path: str, content: str, mtime: str) -> dict:
    """Returns a file entry with a modification time."""
    return {**entry(path, content), "mtime": mtime}


class Coverage:
    """Builder of the coverage cases: one case per row, named by the row."""

    def __init__(self) -> None:
        self.c = Collection("coverage", None, kind="journal")

    def reversible(self, key: str, summary: str, before: dict, program: str, kind: str,
                   after: dict | None = None, requires: tuple[str, ...] = (),
                   config: str | None = None) -> None:
        """Adds a case that performs one operation, undoes it and expects the tree before it."""
        self.c.add(key, summary, before, feature=JOURNAL, requires=requires, config=config,
                   steps=[call(program, report=ok(kind), after=after), undo(1, before)])

    def steps(self, key: str, summary: str, before: dict, steps: list[dict],
              requires: tuple[str, ...] = (), config: str | None = None, feature: str = JOURNAL,
              extra: dict | None = None) -> None:
        """Adds a case with explicit steps; `extra` holds further members of the case header."""
        self.c.add(key, summary, before, feature=feature, requires=requires, config=config, steps=steps,
                   extra=extra)

    def marked(self, key: str, summary: str, program: str, kind: str, reason: str,
               requires: tuple[str, ...] = ()) -> None:
        """Adds a case of one call whose report must name an irreversible reason."""
        self.c.add(key, summary, tree({}), program, feature=JOURNAL, requires=requires,
                   report=irreversible(kind, reason))


def core_rows(b: Coverage) -> None:
    """Rows of the group core: files, links, metadata, programs and the journal commands."""
    text = tree(TEXT)
    dirs = tree(DIRS)
    b.c.add("verify", "A failed verification after a change restores every target of the change.",
            tree({"a.txt": "old\n"}),
            'replace "old" "new" in: ["a.txt"] |> verify (read "a.txt" |> assert contains: "absent")',
            feature=JOURNAL,
            report={"status": "rolled_back", "kind": "replace", "error": {"code": "E_ROLLED_BACK"}})
    b.reversible("write-create", "Undo of a created file removes it.", text,
                 'write "n.txt" "new\\n" mode: create', "write", changed(TEXT, n_dot_txt="new\n"))
    b.reversible("write-overwrite", "Undo of an overwrite restores the previous bytes.", text,
                 'write "a.txt" "replaced\\n" mode: overwrite', "write", changed(TEXT, a_dot_txt="replaced\n"))
    b.reversible("write-append", "Undo of an append restores the previous length.", text,
                 'write "a.txt" "gamma\\n" mode: append', "write", changed(TEXT, a_dot_txt="alpha\nbeta\ngamma\n"))
    b.reversible("write-prepend", "Undo of a prepend restores the previous bytes.", text,
                 'write "a.txt" "zero\\n" mode: prepend', "write", changed(TEXT, a_dot_txt="zero\nalpha\nbeta\n"))
    b.reversible("write-files", "Undo of a batch of writes restores every file of the batch as one operation.", text,
                 'write files: [{path: "${base}/n.txt", text: "new\\n", mode: "create"}, '
                 '{path: "${base}/a.txt", text: "gamma\\n", mode: "append"}, '
                 '{path: "${base}/b.txt", text: "B\\n", mode: "overwrite"}]', "write",
                 changed(TEXT, n_dot_txt="new\n", a_dot_txt="alpha\nbeta\ngamma\n", b_dot_txt="B\n"))
    b.reversible("write-size", "Undo of a truncation restores the removed bytes.", text,
                 'write "a.txt" size: 5', "write", changed(TEXT, a_dot_txt="alpha"))
    b.reversible("write-size/extend", "Undo of an extension restores the previous length.", tree({"a.txt": "ab"}),
                 'write "a.txt" size: 4', "write", tree({"a.txt": b"ab\x00\x00"}))
    windows = ("os:windows",)
    b.steps("write-registry", "Undo of a written registry value removes the value.", tree({}), [
        call(f'write "{REGISTRY}/value" "data" value_type: string', report=ok("write")),
        undo(1, tree({})),
        call(f'info "{REGISTRY}/value"', report={"status": "ok", "records": entry_rows(False)}),
    ], requires=windows)
    b.marked("write-special-file", "A write to a special file is reported as irreversible.",
             'write "/dev/null" "x\\n" mode: append', "write", "special_file", requires=("os:unix",))
    b.reversible("replace", "Undo of a replacement restores the previous bytes.", text,
                 'replace "beta" "BETA" in: ["a.txt"]', "replace", changed(TEXT, a_dot_txt="alpha\nBETA\n"))
    b.reversible("insert", "Undo of an insertion restores the previous bytes.", text,
                 'insert "middle" in: "a.txt" line: 1', "insert", changed(TEXT, a_dot_txt="alpha\nmiddle\nbeta\n"))
    b.reversible("insert/end", "Undo of an insertion at the end restores the previous length.", text,
                 'insert "omega" in: "a.txt" position: end', "insert")
    b.reversible("edit", "Undo of an edit restores the previous bytes.", text,
                 'edit "a.txt" changes: [{replace: "alpha", with: "ALPHA"}, {replace: "beta", with: "BETA"}]',
                 "edit", changed(TEXT, a_dot_txt="ALPHA\nBETA\n"))
    b.reversible("copy", "Undo of a copied tree removes the copy.", dirs, 'copy "dir" to: "dir2"', "copy",
                 changed(DIRS, dir2__c_dot_txt="charlie\n", dir2__sub__d_dot_txt="delta\n"))
    b.reversible("copy/overwrite", "Undo of a copy onto an existing file restores the destination.", dirs,
                 'copy "e.txt" to: "a.txt"', "copy", changed(DIRS, a_dot_txt="echo\n"))
    b.steps("copy-registry", "Undo of a copied registry value removes the copy.", tree({}), [
        call(f'write "{REGISTRY}/value" "data" value_type: string', report=ok("write")),
        call(f'copy "{REGISTRY}/value" to: "{REGISTRY}/copy"', report=ok("copy")),
        undo(2, tree({})),
        call(f'info ["{REGISTRY}/value", "{REGISTRY}/copy"]',
             report={"status": "ok", "records": entry_rows(True, False)}),
        call(f'remove "{REGISTRY}"', setup_only=True),
    ], requires=windows)
    b.reversible("move", "Undo of a move onto an existing file restores both files.", text,
                 'move "b.txt" to: "a.txt"', "move", changed(TEXT, b_dot_txt=REMOVED, a_dot_txt="bravo\n"))
    b.reversible("move/new-name", "Undo of a move to a new name restores the old name.", text,
                 'move "b.txt" to: "m.txt"', "move", changed(TEXT, b_dot_txt=REMOVED, m_dot_txt="bravo\n"))
    b.steps("move-registry", "Undo of a moved registry value restores it under its old name.", tree({}), [
        call(f'write "{REGISTRY}/value" "data" value_type: string', report=ok("write")),
        call(f'move "{REGISTRY}/value" to: "{REGISTRY}/moved"', report=ok("move")),
        undo(2, tree({})),
        call(f'info ["{REGISTRY}/value", "{REGISTRY}/moved"]',
             report={"status": "ok", "records": entry_rows(True, False)}),
        call(f'remove "{REGISTRY}"', setup_only=True),
    ], requires=windows)
    b.reversible("remove", "Undo of a removed directory tree restores every entry.", dirs, 'remove "dir"', "remove",
                 changed(DIRS, dir__c_dot_txt=REMOVED, dir__sub__d_dot_txt=REMOVED))
    b.reversible("remove/files", "Undo of several removed files restores all of them as one operation.", dirs,
                 'remove ["a.txt", "e.txt"]', "remove", changed(DIRS, a_dot_txt=REMOVED, e_dot_txt=REMOVED))
    b.reversible("remove-records", "Undo of removed lines restores the previous bytes.", text,
                 'lines "a.txt" |> filter text == "beta" |> remove', "remove", changed(TEXT, a_dot_txt="alpha\n"))
    b.steps("remove-registry", "Undo of a removed registry value restores it.", tree({}), [
        call(f'write "{REGISTRY}/value" "data" value_type: string', report=ok("write")),
        call(f'remove "{REGISTRY}/value"', report=ok("remove")),
        undo(2, tree({})),
        call(f'info "{REGISTRY}/value"', report={"status": "ok", "records": entry_rows(True)}),
        call(f'remove "{REGISTRY}"', setup_only=True),
    ], requires=windows)
    unix = ("os:unix",)
    b.steps("remove-job", "Removing a finished job is reported as irreversible.", tree({}), [
        call('run ["sh", "-c", "echo done"] cwd: "${base}" background: true'),
        call(f'jobs "{job(1)}" wait: 10s'),
        call(f'jobs "{job(1)}" |> remove', report=irreversible("remove", "job_output_removed")),
    ], requires=unix)
    b.steps("move-cross-volume", "Undo of a move to another volume restores the file on both volumes.", text, [
        call('move "a.txt" to: "${second_volume}/a.txt"', report=ok("move"), after=changed(TEXT, a_dot_txt=REMOVED),
             second_after=tree({"a.txt": TEXT["a.txt"]})),
        call(f"undo {op(1)}", report=UNDONE, after=text, second_after=tree({})),
    ], requires=("fixture:second-volume",), extra={"second_volume_before": tree({})})
    b.steps("create-temporary", "Undo of a created temporary file removes it.", text, [
        call('create "${base}" temporary: true type: file', report=ok("create")),
        undo(1, text),
    ])
    b.reversible("rename", "Undo of a rename restores the old name.", text, 'rename path: "b.txt" to: "r.txt"', "rename",
                 changed(TEXT, b_dot_txt=REMOVED, r_dot_txt="bravo\n"))
    b.reversible("create-file", "Undo of a created file removes it.", text, 'create "n.txt" type: file', "create",
                 changed(TEXT, n_dot_txt=""))
    b.reversible("create-directory", "Undo of created directories removes them, parents included.", text,
                 'create "out/logs" type: directory', "create", changed(TEXT, out__logs=None))
    b.reversible("create-link", "Undo of a created symbolic link removes the link and keeps its target.", text,
                 'create "l" type: link target: "a.txt"', "create",
                 {"entries": tree(TEXT)["entries"] + [entry("l", target="a.txt")]}, requires=unix)
    b.reversible("create-link/hard", "Undo of a created hard link removes the link and keeps the file.", text,
                 'create "h.txt" type: link target: "${base}/a.txt" hard: true', "create",
                 changed(TEXT, h_dot_txt="alpha\nbeta\n"))
    b.reversible("create-junction", "Undo of a created junction removes it and keeps its target.", dirs,
                 'create "j" type: junction target: "${base}/dir"', "create", requires=windows)
    stamped = snapshot(timed("a.txt", "alpha\n", OLD_TIME))
    b.reversible("create-touch", "Undo of a touch restores the previous modification time.", stamped,
                 'create "a.txt" type: file on_exists: touch', "create")
    b.reversible("attributes", "Undo of a mode change restores the previous mode.",
                 snapshot(entry("a.txt", "alpha\n", mode="0644")), 'attributes "a.txt" mode: "0600"', "attributes",
                 snapshot(entry("a.txt", "alpha\n", mode="0600")), requires=unix)
    b.reversible("attributes/modified", "Undo of a changed modification time restores the previous time.", stamped,
                 f'attributes "a.txt" modified: "{NEW_TIME}"', "attributes",
                 snapshot(timed("a.txt", "alpha\n", NEW_TIME)))
    b.reversible("run", "Undo of a program run restores the watched files the program changed.",
                 tree({"a.txt": "old\n"}),
                 'run ["sh", "-c", "echo new > a.txt; echo b > b.txt"] cwd: "${base}" watch: ["${base}"]', "run",
                 tree({"a.txt": "new\n", "b.txt": "b\n"}), requires=unix)
    b.c.add("run/unpreserved", "A watched file changed without a kept copy is reported as irreversible.",
            tree({"a.txt": "old\n"}), 'run ["sh", "-c", "echo new > a.txt"] cwd: "${base}" watch: ["${base}"]',
            feature=JOURNAL, requires=unix, config=NO_PREIMAGE,
            report=irreversible("run", "previous_content_not_kept"), after=tree({"a.txt": "new\n"}))
    b.steps("stop", "Stopping a job is reported as irreversible.", tree({}), [
        call(SLEEPER),
        call(f'jobs "{job(1)}" |> stop', report=irreversible("stop", "process_stopped")),
    ], requires=unix)
    log_once = tree({"log.txt": "x\n"})
    b.steps("retry", "Undo of a retried program restores the files of the retry only.", tree({}), [
        call('run ["sh", "-c", "echo x >> log.txt"] cwd: "${base}" background: true'),
        call(f'jobs "{job(1)}" wait: 10s', after=log_once),
        call(f'jobs "{job(1)}" |> retry wait: forever', report=ok("retry"), after=tree({"log.txt": "x\nx\n"})),
        undo(3, log_once),
    ], requires=unix)
    b.steps("retry/unpreserved", "A retry that changes a file without a kept copy is reported as irreversible.",
            tree({}), [
                call('run ["sh", "-c", "echo x >> log.txt"] cwd: "${base}" background: true'),
                call(f'jobs "{job(1)}" wait: 10s'),
                call(f'jobs "{job(1)}" |> retry wait: forever',
                     report=irreversible("retry", "previous_content_not_kept")),
            ], requires=unix, config=NO_PREIMAGE)
    replaced = changed(TEXT, a_dot_txt="replaced\n")
    b.steps("undo", "The undo entry keeps the restored items, so its redo returns the state after the change.",
            text, [
                call('write "a.txt" "replaced\\n" mode: overwrite', report=ok("write"), after=replaced),
                undo(1, text),
                call(f"redo {op(2)}", report=ok("redo"), after=replaced),
            ])
    b.steps("redo", "Undo of a redo restores the state before the redo.", text, [
        call('write "a.txt" "replaced\\n" mode: overwrite', report=ok("write")),
        undo(1, text),
        call(f"redo {op(2)}", report=ok("redo"), after=replaced),
        undo(3, text),
    ])


def group_rows(b: Coverage) -> None:
    """Rows of the groups documents, text, code, notebook, archive, system, admin and package."""
    b.reversible("set", "Undo of a changed document value restores the previous bytes.", tree(DOCUMENT),
                 'set "c.json" key: "version" value: "1.1.0"', "set",
                 tree({"c.json": '{\n  "name": "app",\n  "version": "1.1.0"\n}\n'}))
    b.steps("sql-write", "Undo of a database change restores the content of the database.", tree({}), [
        call('sql "app.db" "create table t(x integer)" write: true', report=ok("sql")),
        call('sql "app.db" "insert into t values (1)" write: true', report=ok("sql")),
        call(f"undo {op(2)}", report=UNDONE),
        call('sql "app.db" "select x from t"', report=ok("sql", records=records("row", matched=0))),
    ])
    b.reversible("sql-write/create", "Undo of a created database removes the database and its side files.",
                 tree({}), 'sql "app.db" "create table t(x integer)" write: true', "sql")
    b.reversible("patch", "Undo of an applied patch restores the previous bytes.", tree({"a.txt": "one\ntwo\n"}),
                 PATCH_EDIT, "patch", tree({"a.txt": "one\nTWO\n"}))
    b.reversible("patch/files", "Undo of a patch that creates and deletes files restores both.", tree(TEXT),
                 PATCH_FILES, "patch", changed(TEXT, b_dot_txt=REMOVED, n_dot_txt="new\n"))
    b.reversible("convert", "Undo of a line ending conversion restores the previous bytes.",
                 tree({"w.txt": "one\r\ntwo\r\n"}), 'convert "w.txt" line_ending: lf', "convert",
                 tree({"w.txt": "one\ntwo\n"}))
    b.reversible("convert/encoding", "Undo of an encoding conversion restores the previous bytes.",
                 tree({"l.txt": "Žilina\n".encode("cp1250")}), 'convert "l.txt" encoding: "utf-8"', "convert",
                 tree({"l.txt": "Žilina\n"}))
    b.reversible("code.rename", "Undo of a rename across files restores every file.",
                 tree({"a.py": "def old_name():\n    pass\n", "b.py": "from a import old_name\nold_name()\n"}),
                 'code.symbols "old_name" path: ["${base}"] kind: function source: syntax match: exact '
                 '|> code.rename "new_name" source: syntax', "code.rename", requires=("feature:code",))
    notebook_rows = (
        ("notebook.insert", NOTEBOOK_ONE, 'notebook.insert "a.ipynb" index: 0 cell_type: code source: "print(2)"'),
        ("notebook.replace", NOTEBOOK_CODE, 'notebook.replace "a.ipynb" id: "c1" source: "print(9)"'),
        ("notebook.move", NOTEBOOK_TWO, 'notebook.move "a.ipynb" from: 1 to: 0'),
        ("notebook.copy", NOTEBOOK_ONE, 'notebook.copy "a.ipynb" from: 0 to: 1'),
        ("notebook.normalize", NOTEBOOK_OLD, 'notebook.normalize "a.ipynb"'),
        ("notebook.ops", NOTEBOOK_ONE,
         'notebook.ops "a.ipynb" changes: [{op: "insert", index: 1, cell_type: "code", source: "x = 1"}, '
         '{op: "remove", index: 0}]'),
        ("notebook.remove", NOTEBOOK_ONE, 'notebook.remove "a.ipynb" id: "m"'),
        ("notebook.clear-outputs", NOTEBOOK_OUTPUT, 'notebook.clear_outputs "a.ipynb"'),
        ("notebook.export", NOTEBOOK_CODE, 'notebook.export "a.ipynb" to: "a.py" format: script'),
    )
    for key, fixture, program in notebook_rows:
        command = program.split(" ", 1)[0]
        b.reversible(key, f"Undo of {command} restores the notebook byte for byte.", tree(fixture), program, command)
    archive = zip_bytes({"a.txt": "alpha\n"})
    b.reversible("archive.extract", "Undo of an extraction removes the extracted files.",
                 tree({"in.zip": archive}), 'archive.extract "in.zip" into: "out"', "archive.extract",
                 tree({"in.zip": archive, "out/a.txt": "alpha\n"}))
    b.reversible("archive.extract/overwrite", "Undo of an extraction over existing files restores them.",
                 tree({"in.zip": archive, "a.txt": "old\n"}), 'archive.extract "in.zip" into: "${base}"',
                 "archive.extract", tree({"in.zip": archive, "a.txt": "alpha\n"}))
    b.reversible("archive.create", "Undo of a created archive removes it.", tree({"dist/a.txt": "a"}),
                 'find "dist" |> archive.create "out.zip" base: "${base}/dist"', "archive.create")
    b.reversible("compress", "Undo of a compression without a kept source restores the source.",
                 tree({"a.log": "line\n"}), 'compress "a.log" keep: false', "compress")
    b.reversible("decompress", "Undo of a decompression without a kept source restores the compressed file.",
                 tree({"a.log.gz": gzip_bytes(b"line\n")}), 'decompress "a.log.gz" keep: false', "decompress",
                 tree({"a.log": "line\n"}))
    b.steps("sleep", "A command without an effect creates no journal entry.", tree({}), [
        call("sleep 10ms", report=ok("sleep")),
        call("undo", report=failed("E_JOURNAL_NOOP"), after=UNCHANGED),
    ])
    unix = ("os:unix",)
    b.steps("suspend", "Suspending a job is reported as irreversible.", tree({}), [
        call(SLEEPER),
        call(f'jobs "{job(1)}" |> suspend', report=irreversible("suspend", "process_state_changed")),
        call(f'jobs "{job(1)}" |> stop signal: kill', setup_only=True),
    ], requires=unix)
    b.steps("resume", "Resuming a job is reported as irreversible.", tree({}), [
        call(SLEEPER),
        call(f'jobs "{job(1)}" |> suspend'),
        call(f'jobs "{job(1)}" |> resume', report=irreversible("resume", "process_state_changed")),
        call(f'jobs "{job(1)}" |> stop signal: kill', setup_only=True),
    ], requires=unix)
    for command in ("start", "restart", "enable", "disable"):
        b.marked(command, f"The command {command} on a service is reported as irreversible.",
                 f'services name: "{FIXTURE}" |> {command}', command, "service_changed",
                 requires=("feature:services",))
    for command in ("package.install", "package.remove", "package.upgrade"):
        b.marked(command, f"The command {command} is reported as irreversible.", f'{command} ["{FIXTURE}"]',
                 command, "package_changed", requires=("feature:packages",))


def network_rows(b: Coverage) -> None:
    """Rows of the group network, served by the local HTTP server of the runner."""
    http = ("fixture:http",)
    served = {"http_files": tree({"a.txt": "served\n"})}
    text = tree(TEXT)
    b.steps("http.request-save", "Undo of a saved response removes the saved file.", text, [
        call('http.request "${http_base}/files/a.txt" save: "out.txt"', report=ok("http.request"),
             after=changed(TEXT, out_dot_txt="served\n")),
        undo(1, text),
    ], requires=http, extra=served)
    b.steps("http.download", "Undo of a download removes the downloaded file.", text, [
        call('http.download "${http_base}/files/a.txt" "d.txt"', report=ok("http.download"),
             after=changed(TEXT, d_dot_txt="served\n")),
        undo(1, text),
    ], requires=http, extra=served)
    for method, body in (("post", ' json: {name: "a"}'), ("put", ' json: {name: "a"}'),
                         ("patch", ' json: {name: "a"}'), ("delete", ""), ("options", "")):
        b.steps(f"http.request-{method}", f"A {method} request is reported as irreversible and changes no file.",
                text, [
                    call(f'http.request "${{http_base}}/echo/item" method: {method}{body}',
                         report=irreversible("http.request", "network"), after=UNCHANGED),
                ], requires=http)


def schedule_and_tool_rows(b: Coverage) -> None:
    """Rows of the groups schedule, journal and verify."""
    empty = tree({})
    create = 'schedule.create "{label}" every: 1h action: (sleep 1s){extra}'
    b.steps("schedule.create", "Undo of a created schedule removes it.", empty, [
        call(create.format(label="jc", extra=" enabled: false"), report=ok("schedule.create")),
        call(f"undo {op(1)}", report=UNDONE),
        call('schedule.list |> filter label == "jc"', report={"status": "ok", "records": records("schedule", matched=0)}),
    ])
    b.steps("schedule.remove", "Undo of a removed schedule restores it.", empty, [
        call(create.format(label="jr", extra="")),
        call('schedule.list |> filter label == "jr"'),
        call(f'schedule.remove "{schedule(2)}"', report=ok("schedule.remove")),
        call(f"undo {op(3)}", report=UNDONE),
        call('schedule.list |> filter label == "jr"', report={"status": "ok", "records": records("schedule", matched=1)}),
    ])
    b.steps("schedule.pause", "Undo of a paused schedule makes it active again.", empty, [
        call(create.format(label="jp", extra="")),
        call('schedule.list |> filter label == "jp"'),
        call(f'schedule.pause "{schedule(2)}"', report=ok("schedule.pause")),
        call(f"undo {op(3)}", report=UNDONE),
        call('schedule.list state: paused |> filter label == "jp"',
             report={"status": "ok", "records": records("schedule", matched=0)}),
    ])
    b.steps("schedule.resume", "Undo of a resumed schedule pauses it again.", empty, [
        call(create.format(label="js", extra=" enabled: false")),
        call('schedule.list |> filter label == "js"'),
        call(f'schedule.resume "{schedule(2)}"', report=ok("schedule.resume")),
        call(f"undo {op(3)}", report=UNDONE),
        call('schedule.list state: paused |> filter label == "js"',
             report={"status": "ok", "records": records("schedule", matched=1)}),
    ])
    b.steps("schedule.run", "Undo of a run schedule action restores the files the action wrote.", empty, [
        call('schedule.create "jw" every: 1h action: (write "${base}/ran.txt" "ran\\n" mode: overwrite) '
             'enabled: false'),
        call('schedule.list |> filter label == "jw"'),
        call(f'schedule.run "{schedule(2)}"', report=ok("schedule.run"), after=tree({"ran.txt": "ran\n"})),
        undo(3, empty),
    ])
    text = tree(TEXT)
    b.steps("journal.pin", "Undo of a pin restores the previous pin state.", text, [
        call('write "n.txt" "new\\n" mode: create', report=ok("write")),
        call(f"journal.pin [{op(1)}]", report=ok("journal.pin")),
        call(f"undo {op(2)}", report=UNDONE),
        call(f"history {op(1)}", report={"status": "ok", "records": operation(pinned={"t": "bool", "v": False})}),
    ])
    b.steps("journal.pin-release", "Entries that expire after a released pin are reported and cannot be undone.",
            text, [
                call('write "n.txt" "new\\n" mode: create', report=ok("write")),
                call('write "m.txt" "more\\n" mode: create', report=ok("write")),
                call(f"journal.pin [{op(1)}]", report=ok("journal.pin")),
                call("journal.limits steps: 1"),
                call(f"journal.pin [{op(1)}] release: true",
                     report=irreversible("journal.pin", "journal_entries_expired")),
                call(f"undo {op(1)}", report=failed("E_JOURNAL_EXPIRED"), after=UNCHANGED),
                call(f"history {op(1)}", report={"status": "ok", "records": operation(state=enum("expired"))}),
            ])
    b.steps("journal.limits", "Lowered limits expire entries; undo of the limits does not restore them.", text, [
        call('write "n.txt" "new\\n" mode: create', report=ok("write")),
        call('write "m.txt" "more\\n" mode: create', report=ok("write")),
        call("journal.limits steps: 1", report=irreversible("journal.limits", "journal_entries_expired")),
        call(f"undo {op(1)}", report=failed("E_JOURNAL_EXPIRED"), after=UNCHANGED),
        call(f"undo {op(3)}", report=UNDONE),
        call(f"undo {op(1)}", report=failed("E_JOURNAL_EXPIRED"), after=UNCHANGED),
    ])
    b.steps("journal.purge", "A purged entry is reported as irreversible and cannot be undone.", text, [
        call('write "n.txt" "new\\n" mode: create', report=ok("write")),
        call(f"journal.purge id: [{op(1)}]", report=irreversible("journal.purge", "purged")),
        call(f"undo {op(1)}", report=failed("E_JOURNAL_EXPIRED"), after=UNCHANGED),
        call(f"history {op(1)}", report={"status": "ok", "records": operation(state=enum("expired"))}),
    ])
    tests = ("feature:tests", "os:unix")
    script = tree({"t.sh": "echo ok > out.txt\n"})
    b.reversible("test.run", "Undo of a test run removes the files the framework wrote.", script,
                 'test.run "${base}" command: ["sh", "t.sh"] wait: forever', "test.run", requires=tests)
    failing = {"t.sh": "echo ok > out.txt\nexit 1\n"}
    b.steps("test.failed", "Undo of a rerun of failed tests restores the files of the rerun only.", tree(failing), [
        call('test.run "${base}" command: ["sh", "t.sh"] wait: forever'),
        call('test.failed "${step1/report/test/id}" wait: forever', report=ok("test.failed")),
        undo(2, changed(failing, out_dot_txt="ok\n")),
    ], requires=tests)
    analysis = ("feature:analysis",)
    b.reversible("analyze.check", "Undo of an analysis removes the files the analyzers wrote.",
                 tree({"a.py": "x = 1\n"}), 'analyze.check "${base}" wait: forever', "analyze.check",
                 requires=analysis)
    b.reversible("analyze.format", "Undo of formatting restores the source files.", tree({"a.py": "x=1\n"}),
                 'analyze.format "${base}"', "analyze.format", requires=analysis)


def coverage() -> Collection:
    """Returns the coverage cases, one or more per row of the coverage table."""
    b = Coverage()
    core_rows(b)
    group_rows(b)
    network_rows(b)
    schedule_and_tool_rows(b)
    return b.c


def undo_semantics(c: Collection) -> None:
    """Selectors of undo and redo."""
    text = tree(TEXT)
    edited = changed(TEXT, a_dot_txt="alpha\nBETA\n")
    replace = 'replace "beta" "BETA" in: ["a.txt"]'
    c.add("undo-last", "Undo without a selector undoes the newest operation.", text, feature=JOURNAL, steps=[
        call(replace, report=ok("replace"), after=edited),
        call("undo", report=UNDONE, after=text),
    ])
    c.add("undo-last-skips-undo-entries", "Undo without a selector never undoes an undo entry.", text,
          feature=JOURNAL, steps=[
              call('write "n.txt" "new\\n" mode: create', report=ok("write")),
              call(replace, report=ok("replace")),
              call("undo", report=UNDONE, after=changed(TEXT, n_dot_txt="new\n")),
              call("undo", report=UNDONE, after=text),
          ])
    c.add("undo-op", "Undo of an operation by number leaves later operations in place.", text, feature=JOURNAL,
          steps=[
              call(replace, report=ok("replace")),
              call('write "x.txt" "x\\n" mode: create', report=ok("write")),
              undo(1, changed(TEXT, x_dot_txt="x\n")),
          ])
    c.add("undo-report-has-operation-number", "The report of an undo carries its own operation number.", text,
          feature=JOURNAL, steps=[
              call(replace, report=ok("replace")),
              call(f"undo {op(1)}", report=ok("undo", op="<volatile>"), after=text),
          ])
    c.add("undo-bulk-as-whole", "Undo of a change of several files restores every file.", text, feature=JOURNAL,
          steps=[
              call('replace "a" "A" in: ["a.txt", "b.txt"]', report=ok("replace", targets={"count": 2})),
              call("undo", report=UNDONE, after=text),
          ])
    c.add("undo-to", "Undo to an operation reverts every later operation as one undo.", text, feature=JOURNAL,
          steps=[
              call('write "x.txt" "x\\n" mode: create', report=ok("write")),
              call(replace, report=ok("replace")),
              call('remove "b.txt"', report=ok("remove")),
              call(f"undo to: {op(1)}", report=UNDONE, after=changed(TEXT, x_dot_txt="x\n")),
              call("history limit: 1", report={"status": "ok", "records": operation(undo_scope=enum("to"))}),
          ])
    c.add("undo-file", "Undo of one file restores only that file.", text, feature=JOURNAL, steps=[
        call('replace "a" "A" in: ["a.txt", "b.txt"]', report=ok("replace")),
        call('undo file: "a.txt"', report=UNDONE, after=changed(TEXT, b_dot_txt="brAvo\n")),
        call(f"history {op(1)}", report={"status": "ok", "records": operation(state=enum("partial"))}),
    ])
    c.add("undo-paths", "Undo of selected paths of one operation restores those paths only.", text,
          feature=JOURNAL, steps=[
              call('remove ["a.txt", "b.txt"]', report=ok("remove")),
              call(f'undo {op(1)} paths: ["b.txt"]', report=UNDONE, after=changed(TEXT, a_dot_txt=REMOVED)),
              call(f"history {op(1)}", report={"status": "ok", "records": operation(state=enum("partial"))}),
          ])
    c.add("redo", "Redo of an undo applies the operation again.", text, feature=JOURNAL, steps=[
        call(replace, report=ok("replace")),
        call("undo", report=UNDONE, after=text),
        call(f"redo {op(2)}", report=ok("redo"), after=edited),
    ])
    c.add("redo-invalidated", "Redo after a later change of the same file reports the conflict.", text,
          feature=JOURNAL, steps=[
              call(replace, report=ok("replace")),
              call("undo", report=UNDONE),
              call('write "a.txt" "x\\n" mode: overwrite', report=ok("write")),
              call(f"redo {op(2)}", report=failed("E_JOURNAL_CONFLICT"), after=UNCHANGED),
          ])
    c.add("redo-of-change", "Redo of an operation that is not an undo reports nothing to redo.", text,
          feature=JOURNAL, steps=[
              call(replace, report=ok("replace")),
              call(f"redo {op(1)}", report=failed("E_JOURNAL_NOOP"), after=UNCHANGED),
          ])
    c.add("undo-nothing", "Undo with an empty journal reports nothing to undo.", text, "undo", feature=JOURNAL,
          report=failed("E_JOURNAL_NOOP"))
    c.add("undo-unknown-operation", "Undo of an unknown operation changes nothing.", text, "undo 999999",
          feature=JOURNAL, report=failed("E_JOURNAL_NOOP"))
    c.add("undo-twice", "A second undo of the same operation reports nothing to undo.", text, feature=JOURNAL,
          steps=[
              call(replace, report=ok("replace")),
              undo(1, text),
              call(f"undo {op(1)}", report=failed("E_JOURNAL_NOOP"), after=UNCHANGED),
          ])
    c.add("undo-after-preview", "A preview creates no operation, so there is nothing to undo.", text,
          feature=JOURNAL, steps=[
              call('remove "b.txt"', options={"preview": True}, report={"status": "planned"}),
              call("undo", report=failed("E_JOURNAL_NOOP"), after=UNCHANGED),
          ])


def conflict_semantics(c: Collection) -> None:
    """Conflict check of undo and the choices of `on_conflict`."""
    text = tree(TEXT)
    replace = 'replace "beta" "BETA" in: ["a.txt"]'
    later = 'write "a.txt" "later\\n" mode: append'
    conflicted = changed(TEXT, a_dot_txt="alpha\nBETA\nlater\n")
    c.add("conflict-report", "Undo after a later change of the same file fails and changes nothing.", text,
          feature=JOURNAL, steps=[
              call(replace, report=ok("replace")),
              call(later, report=ok("write"), after=conflicted),
              call(f"undo {op(1)}", report=failed("E_JOURNAL_CONFLICT"), after=UNCHANGED),
          ])
    c.add("conflict-report-explicit", "The choice report is the default of on_conflict.", text, feature=JOURNAL,
          steps=[
              call(replace, report=ok("replace")),
              call(later, report=ok("write")),
              call(f"undo {op(1)} on_conflict: report", report=failed("E_JOURNAL_CONFLICT"), after=UNCHANGED),
          ])
    c.add("conflict-force", "Undo with force restores the content despite a later change.", text, feature=JOURNAL,
          steps=[
              call(replace, report=ok("replace")),
              call(later, report=ok("write")),
              call(f"undo {op(1)} on_conflict: force", report=UNDONE, after=text),
          ])
    c.add("conflict-force-redo", "Undo with force keeps the current content, so its redo restores it.", text,
          feature=JOURNAL, steps=[
              call(replace, report=ok("replace")),
              call(later, report=ok("write")),
              call(f"undo {op(1)} on_conflict: force", report=UNDONE, after=text),
              call(f"redo {op(3)}", report=ok("redo"), after=conflicted),
          ])
    c.add("conflict-include-later", "Undo with include_later also undoes the later change.", text, feature=JOURNAL,
          steps=[
              call(replace, report=ok("replace")),
              call(later, report=ok("write")),
              call(f"undo {op(1)} on_conflict: include_later", report=UNDONE, after=text),
          ])
    c.add("conflict-abort", "Undo with abort changes nothing on a conflict.", text, feature=JOURNAL, steps=[
        call(replace, report=ok("replace")),
        call(later, report=ok("write")),
        call(f"undo {op(1)} on_conflict: abort", after=UNCHANGED),
    ])
    c.add("conflict-partial", "Undo of a partial entry fails unless partial asks for the remaining items.", text,
          feature=JOURNAL, steps=[
              call('remove ["a.txt", "b.txt"]', report=ok("remove")),
              call(f'undo {op(1)} paths: ["b.txt"]', report=UNDONE),
              call(f"undo {op(1)}", report=failed("E_JOURNAL_PARTIAL"), after=UNCHANGED),
              call(f"undo {op(1)} on_conflict: partial", report=UNDONE, after=text),
          ])


def history_semantics(c: Collection) -> None:
    """History of the journal and the absence of expiry information in results."""
    text = tree(TEXT)
    c.add("history-newest-first", "History lists the operations, newest first.", text, feature=JOURNAL, steps=[
        call('write "x.txt" "x\\n" mode: create', report=ok("write")),
        call('remove "b.txt"', report=ok("remove")),
        call("history limit: 2", report=ok("history", records=records("operation", [
            row("operation", kind="remove"), row("operation", kind="write")]))),
    ])
    c.add("history-state-undone", "History shows an undone operation as undone.", text, feature=JOURNAL, steps=[
        call('remove "b.txt"', report=ok("remove")),
        call("undo", report=UNDONE),
        call(f"history {op(1)}", report={"status": "ok", "records": operation(state=enum("undone"))}),
    ])
    c.add("history-state-filter", "History filtered by state lists only the matching operations.", text,
          feature=JOURNAL, steps=[
              call('write "x.txt" "x\\n" mode: create', report=ok("write")),
              call('remove "b.txt"', report=ok("remove")),
              call("undo", report=UNDONE),
              call("history state: undone", report={"status": "ok", "records": records("operation", matched=1)}),
          ])
    c.add("history-of-path", "History of one path lists only the operations that touched it.", text,
          feature=JOURNAL, steps=[
              call('write "x.txt" "x\\n" mode: create', report=ok("write")),
              call('remove "b.txt"', report=ok("remove")),
              call('history path: "b.txt"', report={"status": "ok", "records": records("operation", matched=1)}),
          ])
    c.add("history-reads-create-no-entry", "Reading programs create no journal entry.", text, feature=JOURNAL,
          steps=[
              call('write "x.txt" "x\\n" mode: create', report=ok("write")),
              call('read "a.txt"', report={"status": "ok"}),
              call('find "."', report={"status": "ok"}),
              call("history", report={"status": "ok", "records": records("operation", matched=1)}),
          ])
    c.add("history-undo-scope", "History records the scope of an undo.", text, feature=JOURNAL, steps=[
        call('remove "b.txt"', report=ok("remove")),
        call(f"undo {op(1)}", report=UNDONE),
        call(f"history {op(2)}", report={"status": "ok", "records": operation(undo_scope=enum("op"))}),
    ])
    c.add("no-expiry-in-change-report", "The report of a change has the operation number and no expiry member.",
          text, feature=JOURNAL, steps=[
              call('remove "b.txt"', expect={"status": "ok", "op": "<volatile>"}, expect_at="/report",
                   validate="report", after=changed(TEXT, b_dot_txt=REMOVED), absent=NO_JOURNAL_MEMBERS),
          ])
    c.add("no-expiry-in-undo-report", "The report of an undo has no expiry member.", text, feature=JOURNAL, steps=[
        call('remove "b.txt"', report=ok("remove")),
        call(f"undo {op(1)}", expect={"status": "ok", "kind": "undo"}, expect_at="/report", validate="report",
             after=text, absent=NO_JOURNAL_MEMBERS),
    ])
    c.add("no-expiry-in-history-report", "The report of a history query has no expiry member.", text,
          feature=JOURNAL, steps=[
              call('remove "b.txt"', report=ok("remove")),
              call(f"history {op(1)} details: true", expect={"status": "ok"}, expect_at="/report",
                   validate="report", absent=NO_EXPIRY_FIELDS),
          ])


def expiry_semantics(c: Collection) -> None:
    """Expiry by the retention limits and an inactive journal."""
    text = tree(TEXT)
    c.add("expiry-steps", "An entry beyond the step limit expires and cannot be undone.", text, feature=JOURNAL,
          steps=[
              call("journal.limits steps: 2", report=ok("journal.limits")),
              call('write "x.txt" "x\\n" mode: create', report=ok("write")),
              call('write "y.txt" "y\\n" mode: create', report=ok("write")),
              call('write "z.txt" "z\\n" mode: create', report=ok("write")),
              call(f"undo {op(2)}", report=failed("E_JOURNAL_EXPIRED"), after=UNCHANGED),
              call(f"history {op(2)}", report={"status": "ok", "records": operation(state=enum("expired"))}),
          ])
    c.add("expiry-in-selection", "Undo to an operation fails and changes nothing when the selection has an "
          "expired entry.", text, feature=JOURNAL, steps=[
              call("journal.limits steps: 2", report=ok("journal.limits")),
              call('write "x.txt" "x\\n" mode: create', report=ok("write")),
              call('write "y.txt" "y\\n" mode: create', report=ok("write")),
              call('write "z.txt" "z\\n" mode: create', report=ok("write")),
              call(f"undo to: {op(1)}", report=failed("E_JOURNAL_EXPIRED"), after=UNCHANGED),
          ])
    c.add("expiry-age", "An entry older than the age limit expires on the test clock.", text, feature="testkit",
          requires=("feature:journal",), steps=[
              call("journal.limits age: 1h", report=ok("journal.limits")),
              call('write "x.txt" "x\\n" mode: create', report=ok("write")),
              call(method="testkit.clock.advance", params={"by": "2h"}),
              call(f"undo {op(2)}", report=failed("E_JOURNAL_EXPIRED"), after=UNCHANGED),
              call(f"history {op(2)}", report={"status": "ok", "records": operation(state=enum("expired"))}),
          ])
    c.add("expiry-age-within-limit", "An entry younger than the age limit can still be undone on the test clock.",
          text, feature="testkit", requires=("feature:journal",), steps=[
              call("journal.limits age: 1h", report=ok("journal.limits")),
              call('write "x.txt" "x\\n" mode: create', report=ok("write")),
              call(method="testkit.clock.advance", params={"by": "30m"}),
              undo(2, text),
          ])
    c.add("expiry-age-pinned", "A pinned entry survives the age limit and can be undone after the clock advanced.",
          text, feature="testkit", requires=("feature:journal",), steps=[
              call("journal.limits age: 1h", report=ok("journal.limits")),
              call('write "x.txt" "x\\n" mode: create', report=ok("write")),
              call(f"journal.pin [{op(2)}]", report=ok("journal.pin")),
              call(method="testkit.clock.advance", params={"by": "2h"}),
              undo(2, text),
          ])
    halves = {"x.txt": HALF_BUDGET, "y.txt": HALF_BUDGET}
    c.add("expiry-size", "The oldest entry expires when the kept content exceeds the size limit.", tree(halves),
          feature=JOURNAL, steps=[
              call("journal.limits size: 1kib", report=ok("journal.limits")),
              call('write "x.txt" "new\\n" mode: overwrite', report=ok("write")),
              call('write "y.txt" "new\\n" mode: overwrite', report=ok("write")),
              call(f"undo {op(2)}", report=failed("E_JOURNAL_EXPIRED"), after=UNCHANGED),
              undo(3, tree({"x.txt": "new\n", "y.txt": HALF_BUDGET})),
          ])
    c.add("pin-protects-from-limits", "A pinned entry survives lowered limits and can still be undone.", text,
          feature=JOURNAL, steps=[
              call('write "n.txt" "new\\n" mode: create', report=ok("write")),
              call(f"journal.pin [{op(1)}]", report=ok("journal.pin")),
              call('write "m.txt" "more\\n" mode: create', report=ok("write")),
              call("journal.limits steps: 1", report=irreversible("journal.limits", "journal_entries_expired")),
              undo(1, changed(TEXT, m_dot_txt="more\n")),
          ])
    c.add("pin-release-within-limits", "Releasing a pin within the limits expires nothing; the entry can be undone.",
          text, feature=JOURNAL, steps=[
              call('write "n.txt" "new\\n" mode: create', report=ok("write")),
              call(f"journal.pin [{op(1)}]", report=ok("journal.pin")),
              call(f"journal.pin [{op(1)}] release: true", report=ok("journal.pin")),
              undo(1, text),
          ])
    c.add("journal-inactive", "A change with the journal disabled is reported as irreversible and cannot be undone.",
          text, feature=JOURNAL, config=JOURNAL_DISABLED, steps=[
              call('remove "b.txt"', report=irreversible("remove", "journal_inactive"),
                   after=changed(TEXT, b_dot_txt=REMOVED)),
              call("undo", report=failed("E_JOURNAL_NOOP"), after=UNCHANGED),
          ])
    c.add("journal-below-reserve", "A change on a volume below the space reserve is made without a copy and "
          "reported as no_space_for_copy.", text, feature=JOURNAL, config=BELOW_RESERVE, steps=[
              call('remove "b.txt"', report=irreversible("remove", "no_space_for_copy"),
                   after=changed(TEXT, b_dot_txt=REMOVED)),
          ])
    big = {"big.txt": BIG}
    c.add("volume-max-size-skip", "An operation larger than the size budget of the volume holding the work "
          "directory is made without a copy.", tree(big), feature=JOURNAL, config=VOLUME_MAX_SIZE, steps=[
              call('remove "big.txt"', report=irreversible("remove", "too_large_for_journal"), after=tree({})),
          ])
    c.add("volume-max-size-expiry", "The size budget of a volume expires the oldest entry of that volume.",
          tree(halves), feature=JOURNAL, config=VOLUME_MAX_SIZE, steps=[
              call('write "x.txt" "new\\n" mode: overwrite', report=ok("write")),
              call('write "y.txt" "new\\n" mode: overwrite', report=ok("write")),
              call(f"undo {op(1)}", report=failed("E_JOURNAL_EXPIRED"), after=UNCHANGED),
              undo(2, tree({"x.txt": "new\n", "y.txt": HALF_BUDGET})),
          ])
    c.add("journal-op-too-large", "An operation larger than the limit of one entry is made without a copy and "
          "reported as too_large_for_journal.", tree(big), feature=JOURNAL, config=OP_SIZE_LIMIT, steps=[
              call('remove "big.txt"', report=irreversible("remove", "too_large_for_journal"), after=tree({})),
          ])
    c.add("journal-oversize-skip", "An operation larger than the size budget is made without a copy by default.",
          tree(big), feature=JOURNAL, config=SIZE_BUDGET, steps=[
              call('remove "big.txt"', report=irreversible("remove", "too_large_for_journal"), after=tree({})),
          ])
    c.add("journal-oversize-keep", "With oversize keep, an operation larger than the size budget is kept and can "
          "be undone.", tree(big), feature=JOURNAL, config=SIZE_BUDGET_KEEP, steps=[
              call('remove "big.txt"', report=ok("remove"), after=tree({})),
              undo(1, tree(big)),
          ])


def semantics() -> Collection:
    """Returns the cases of the undo semantics."""
    c = Collection("semantics", None, kind="journal")
    undo_semantics(c)
    conflict_semantics(c)
    history_semantics(c)
    expiry_semantics(c)
    return c


def collections() -> list[Collection]:
    """Returns every journal collection."""
    return [coverage(), semantics()]
