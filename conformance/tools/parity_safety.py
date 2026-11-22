# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Parity scenarios of the safety tools: undo, redo, transactions, checks and unusual effects.

Scenarios that undo or redo need the journal and belong to the class
`optional:journal`; they are mandatory for implementations that declare it.
"""

from __future__ import annotations

from parity_model import REMOVED, UNCHANGED, Collection, changed, records, row, step, tree

BASE = {
    "a.txt": "alpha\nbeta\n",
    "b.txt": "bravo\n",
    "dir/c.txt": "charlie\n",
    "dir/sub/d.txt": "delta\n",
    "c.json": '{\n  "name": "app",\n  "version": "1.0.0"\n}\n',
    "w.txt": "one\r\ntwo\r\n",
}

CHANGES = [
    ("write-create", 'write "new.txt" "new\\n" mode: create', dict(new_dot_txt="new\n")),
    ("write-overwrite", 'write "a.txt" "replaced\\n" mode: overwrite', dict(a_dot_txt="replaced\n")),
    ("write-append", 'write "a.txt" "gamma\\n" mode: append', dict(a_dot_txt="alpha\nbeta\ngamma\n")),
    ("write-prepend", 'write "a.txt" "zero\\n" mode: prepend', dict(a_dot_txt="zero\nalpha\nbeta\n")),
    ("replace", 'replace "beta" "BETA" in: ["a.txt"]', dict(a_dot_txt="alpha\nBETA\n")),
    ("replace-crlf", 'replace "two" "TWO" in: ["w.txt"]', dict(w_dot_txt="one\r\nTWO\r\n")),
    ("insert", 'insert "middle" in: "a.txt" line: 1', dict(a_dot_txt="alpha\nmiddle\nbeta\n")),
    ("edit", 'edit "a.txt" changes: [{replace: "alpha", with: "ALPHA"}, {replace: "beta", with: "BETA"}]',
     dict(a_dot_txt="ALPHA\nBETA\n")),
    ("remove-file", 'remove "b.txt"', dict(b_dot_txt=REMOVED)),
    ("remove-tree", 'remove "dir"', dict(dir__c_dot_txt=REMOVED, dir__sub__d_dot_txt=REMOVED)),
    ("move", 'move "b.txt" to: "moved.txt"', dict(b_dot_txt=REMOVED, moved_dot_txt="bravo\n")),
    ("move-overwrite", 'move "b.txt" to: "a.txt"', dict(b_dot_txt=REMOVED, a_dot_txt="bravo\n")),
    ("copy", 'copy "dir" to: "dir2"', dict(dir2__c_dot_txt="charlie\n", dir2__sub__d_dot_txt="delta\n")),
    ("rename", 'rename path: "b.txt" to: "renamed.txt"', dict(b_dot_txt=REMOVED, renamed_dot_txt="bravo\n")),
    ("create-directory", 'create "out/logs" type: directory', dict(out__logs=None)),
    ("set-key", 'set "c.json" key: "version" value: "1.1.0"',
     dict(c_dot_json='{\n  "name": "app",\n  "version": "1.1.0"\n}\n')),
    ("convert", 'convert "w.txt" line_ending: lf', dict(w_dot_txt="one\ntwo\n")),
    ("lines-remove", 'lines "a.txt" |> filter text == "beta" |> remove', dict(a_dot_txt="alpha\n")),
    ("bulk-replace", 'replace "a" "A" in: ["."] glob: "**/*.txt"',
     dict(a_dot_txt="AlphA\nbetA\n", b_dot_txt="brAvo\n", dir__c_dot_txt="chArlie\n", dir__sub__d_dot_txt="deltA\n")),
]
BULK_FILES = 4
BULK_REPLACEMENTS = 6


def after(updates: dict) -> dict:
    """Returns the tree after one of the changes."""
    return changed(BASE, **updates)


def undo_scenarios() -> Collection:
    """Undo and redo of every kind of change restore the exact bytes."""
    c = Collection("safety", "undo")
    fs = tree(BASE)
    for name, program, updates in CHANGES:
        c.add(f"undo-last-{name}", f"Undo of the last operation restores the tree after {name}.", fs, feature="journal",
              steps=[step(program, report={"status": "ok"}, after=after(updates)),
                     step("undo", report={"status": "ok", "kind": "undo"}, after=UNCHANGED)])
        c.add(f"undo-operation-{name}", f"Undo by operation number restores the tree after {name}.", fs, feature="journal",
              steps=[step(program, report={"status": "ok"}, after=after(updates)),
                     step('write "other.txt" "o\\n" mode: create', report={"status": "ok"}),
                     step("undo ${step1/report/op}", report={"status": "ok"},
                          after=changed(BASE, other_dot_txt="o\n"))])
    for name, program, updates in CHANGES[:10]:
        c.add(f"redo-{name}", f"Redo after undo applies {name} again.", fs, feature="journal",
              steps=[step(program, report={"status": "ok"}),
                     step("undo", report={"status": "ok"}),
                     step("redo ${step2/report/op}", report={"status": "ok", "kind": "redo"}, after=after(updates))])
    return c


def journal_scenarios() -> Collection:
    """Selections, conflicts and the journal history."""
    c = Collection("safety", "journal")
    fs = tree(BASE)
    edited = dict(a_dot_txt="alpha\nBETA\n")
    c.add("undo-file", "Undo of one file restores only that file.", fs, feature="journal",
          steps=[step('replace "a" "A" in: ["a.txt", "b.txt"]', report={"status": "ok"}),
                 step('undo file: "a.txt"', report={"status": "ok"},
                      after=changed(BASE, b_dot_txt="brAvo\n"))])
    c.add("undo-paths", "Undo of selected paths of one operation restores those paths.", fs, feature="journal",
          steps=[step('remove ["a.txt", "b.txt"]', report={"status": "ok"}),
                 step('undo ${step1/report/op} paths: ["b.txt"]', report={"status": "ok"},
                      after=changed(BASE, a_dot_txt=REMOVED))])
    c.add("undo-to", "Undo to an operation reverts every later operation.", fs, feature="journal",
          steps=[step('write "x.txt" "x\\n" mode: create', report={"status": "ok"}),
                 step('replace "beta" "BETA" in: ["a.txt"]', report={"status": "ok"}),
                 step('remove "b.txt"', report={"status": "ok"}),
                 step("undo to: ${step1/report/op}", report={"status": "ok"},
                      after=changed(BASE, x_dot_txt="x\n"))])
    c.add("conflict-report", "Undo after a later change of the same file reports the conflict and changes nothing.",
          fs, feature="journal",
          steps=[step('replace "beta" "BETA" in: ["a.txt"]', report={"status": "ok"}),
                 step('write "a.txt" "later\\n" mode: append', report={"status": "ok"}),
                 step("undo ${step1/report/op}", report={"status": "failed", "error": {"code": "E_JOURNAL_CONFLICT"}},
                      after=changed(BASE, a_dot_txt="alpha\nBETA\nlater\n"))])
    c.add("conflict-force", "Undo with force restores the content despite a later change.", fs, feature="journal",
          steps=[step('replace "beta" "BETA" in: ["a.txt"]', report={"status": "ok"}),
                 step('write "a.txt" "later\\n" mode: append', report={"status": "ok"}),
                 step("undo ${step1/report/op} on_conflict: force", report={"status": "ok"}, after=tree(BASE))])
    c.add("conflict-include-later", "Undo with include_later also undoes the later change.", fs, feature="journal",
          steps=[step('replace "beta" "BETA" in: ["a.txt"]', report={"status": "ok"}),
                 step('write "a.txt" "later\\n" mode: append', report={"status": "ok"}),
                 step("undo ${step1/report/op} on_conflict: include_later", report={"status": "ok"}, after=tree(BASE))])
    c.add("conflict-abort", "Undo with abort changes nothing on a conflict.", fs, feature="journal",
          steps=[step('replace "beta" "BETA" in: ["a.txt"]', report={"status": "ok"}),
                 step('write "a.txt" "later\\n" mode: append', report={"status": "ok"}),
                 step("undo ${step1/report/op} on_conflict: abort",
                      after=changed(BASE, a_dot_txt="alpha\nBETA\nlater\n"))])
    c.add("redo-invalidated", "Redo after a later change of the same file reports the conflict.", fs, feature="journal",
          steps=[step('replace "beta" "BETA" in: ["a.txt"]', report={"status": "ok"}),
                 step("undo", report={"status": "ok"}),
                 step('write "a.txt" "x\\n" mode: overwrite', report={"status": "ok"}),
                 step("redo ${step2/report/op}", report={"status": "failed", "error": {"code": "E_JOURNAL_CONFLICT"}},
                      after=changed(BASE, a_dot_txt="x\n"))])
    c.add("undo-nothing", "Undo without operations reports that there is nothing to undo.", fs, feature="journal",
          steps=[step("undo", report={"status": "failed", "error": {"code": "E_JOURNAL_NOOP"}}, after=UNCHANGED)])
    c.add("undo-unknown-operation", "Undo of an unknown operation changes nothing.", fs, feature="journal",
          steps=[step("undo 999999", report={"status": "failed", "error": {"code": "E_JOURNAL_NOOP"}}, after=UNCHANGED)])
    c.add("history-lists-operations", "History lists the operations, newest first.", fs, feature="journal",
          steps=[step('write "x.txt" "x\\n" mode: create', report={"status": "ok"}),
                 step('remove "b.txt"', report={"status": "ok"}),
                 step("history limit: 2", report={"status": "ok", "kind": "history", "records": records("operation", [
                     row("operation", kind="remove"), row("operation", kind="write")])})])
    c.add("history-state-undone", "History shows an undone operation as undone.", fs, feature="journal",
          steps=[step('remove "b.txt"', report={"status": "ok"}),
                 step("undo", report={"status": "ok"}),
                 step("history ${step1/report/op}", report={"status": "ok", "records": records("operation", [
                     {"kind": "operation", "fields": {"state": {"t": "enum", "v": "undone"}}}])})])
    c.add("history-of-path", "History of one path lists only the operations that touched it.", fs, feature="journal",
          steps=[step('write "x.txt" "x\\n" mode: create', report={"status": "ok"}),
                 step('remove "b.txt"', report={"status": "ok"}),
                 step('history path: "b.txt"', report={"status": "ok", "records": records("operation", matched=1)})])
    c.add("report-has-no-expiry", "The report of a change carries the operation number and no expiry.", fs,
          'remove "b.txt"', feature="journal", report={"status": "ok", "op": "<volatile>"},
          after=changed(BASE, b_dot_txt=REMOVED))
    c.add("edited-then-undone-twice", "A second undo of the same operation reports nothing to undo.", fs, feature="journal",
          steps=[step('replace "beta" "BETA" in: ["a.txt"]', report={"status": "ok"}, after=after(edited)),
                 step("undo ${step1/report/op}", report={"status": "ok"}, after=tree(BASE)),
                 step("undo ${step1/report/op}", report={"status": "failed", "error": {"code": "E_JOURNAL_NOOP"}},
                      after=tree(BASE))])
    return c


def transaction_scenarios() -> Collection:
    """All-or-nothing changes, checks before the effect and verification after it."""
    c = Collection("safety", "transactions")
    fs = tree(BASE)
    c.add("edit-all-or-nothing", "An edit whose second change fails changes nothing.", fs,
          'edit "a.txt" changes: [{replace: "alpha", with: "A"}, {replace: "absent", with: "B"}] on_missing: error',
          report={"status": "failed", "error": {"code": "E_NO_MATCH"}})
    c.add("bulk-is-one-operation", "A change of several files is one operation.", fs,
          'replace "a" "A" in: ["."] glob: "**/*.txt"', report={"status": "ok", "targets": {"count": BULK_FILES}},
          after=after(CHANGES[-1][2]))
    c.add("verify-passes", "A change whose verification passes stays.", fs,
          'replace "beta" "BETA" in: ["a.txt"] |> verify (read "a.txt" |> assert contains: "BETA")',
          report={"status": "ok"}, after=after({"a_dot_txt": "alpha\nBETA\n"}), feature="journal")
    c.add("verify-fails-rolls-back", "A change whose verification fails is rolled back.", fs,
          'replace "beta" "BETA" in: ["a.txt"] |> verify (read "a.txt" |> assert contains: "gamma")',
          report={"status": "rolled_back", "error": {"code": "E_ROLLED_BACK"}}, feature="journal")
    c.add("verify-fails-bulk", "A failed verification rolls back every file of a bulk change.", fs,
          'replace "a" "A" in: ["."] glob: "**/*.txt" |> verify (find "." glob: "**/*.txt" |> assert empty: true)',
          report={"status": "rolled_back"}, feature="journal")
    c.add("assert-before-holds", "An assertion on the targets before the change holds.", fs,
          'find "." glob: "**/*.txt" |> assert count: 5 |> filter name == "b.txt" |> remove',
          report={"status": "ok"}, after=after({"b_dot_txt": REMOVED}))
    c.add("assert-before-fails", "A failed assertion before the change changes nothing.", fs,
          'find "." glob: "**/*.txt" |> assert count: 3 |> remove',
          report={"status": "failed", "error": {"code": "E_ASSERT", "stage": "3"}})
    c.add("assert-paths", "An assertion on the number of files of a search holds before a replacement.", fs,
          f'search "a" "." glob: "**/*.txt" |> assert paths: {BULK_FILES} |> replace "A"', report={"status": "ok"},
          after=after(CHANGES[-1][2]))
    c.add("assert-paths-fails", "A wrong expected number of files changes nothing.", fs,
          'search "a" "." glob: "**/*.txt" |> assert paths: 1 |> replace "A"',
          report={"status": "failed", "error": {"code": "E_ASSERT"}})
    c.add("assert-empty", "An assertion that nothing matched holds.", fs, 'find "." glob: "**/*.tmp" |> assert empty: true',
          report={"status": "ok"})
    c.add("assert-after-effect", "An assertion after the effect keeps the effect and reports the error.", fs,
          'remove "b.txt" |> assert count: 5', report={"status": "ok", "error": {"code": "E_ASSERT", "stage": "5a"}},
          after=after({"b_dot_txt": REMOVED}))
    c.add("replace-expect-range", "A replacement with an expected range of replacements applies.", fs,
          'replace "a" "A" in: ["a.txt"] expect: 1..5', report={"status": "ok"},
          after=after({"a_dot_txt": "AlphA\nbetA\n"}))
    c.add("stale-read", "A file changed between the read and the change is not changed again.", fs,
          steps=[step('read "a.txt"', report={"status": "ok"}),
                 step('write "a.txt" "external\\n" mode: overwrite', report={"status": "ok"}),
                 step('replace "alpha" "A" in: ["a.txt"] if_match: "${step1/report/records/rows/0/fields/etag/v}"',
                      report={"status": "failed", "error": {"code": "E_STALE"}},
                      after=after({"a_dot_txt": "external\n"}))])
    c.add("one-effect", "Two changing stages in one program change nothing.", fs, 'remove "a.txt" |> remove',
          report={"status": "failed", "error": {"code": "E_ONE_EFFECT"}})
    return c


def check_scenarios() -> Collection:
    """Validation of structured documents and unusual effects of changes."""
    c = Collection("safety", "checks")
    fs = tree(BASE)
    c.add("validate-json-reject", "A change that breaks a JSON document is rejected.", fs,
          'replace "\\"app\\"," "\\"app\\"" in: ["c.json"]', report={"status": "failed", "error": {"code": "E_DOC_PARSE"}})
    c.add("validate-json-warn", "With validate warn a breaking change applies and is reported.", fs,
          'replace "\\"app\\"," "\\"app\\"" in: ["c.json"] validate: warn', report={"status": "ok"},
          after=after({"c_dot_json": '{\n  "name": "app"\n  "version": "1.0.0"\n}\n'}))
    c.add("validate-toml-reject", "A write of a broken TOML document is rejected.", fs,
          'write "c.toml" "[a\\nb = \\n" mode: create', report={"status": "failed", "error": {"code": "E_DOC_PARSE"}})
    c.add("validate-yaml-reject", "A write of a broken YAML document is rejected.", fs,
          'write "c.yaml" "a: [1, 2\\n" mode: create', report={"status": "failed", "error": {"code": "E_DOC_PARSE"}})
    c.add("validate-off", "With validate off any text is written.", fs, 'write "c.yaml" "a: [1, 2\\n" mode: create validate: off',
          report={"status": "ok"}, after=after({"c_dot_yaml": "a: [1, 2\n"}))
    many = {f"logs/{i:03d}.log": "x\n" for i in range(120)}
    c.add("mass-delete", "Removing many files reports a mass deletion.", tree(many), 'remove "logs"',
          report={"status": "ok", "notable": [{"code": "mass_delete", "severity": "warning"}, {"code": "delete_tree"}]},
          after=tree({}))
    c.add("bulk-report", "A change of many targets is reported as a bulk operation.", tree(many),
          'find "logs" glob: "*.log" |> remove', report={"status": "ok", "targets": {"count": 120}},
          after=tree({"logs": None}))
    c.add("delete-tree", "Removing a non-empty directory reports a tree deletion.", fs, 'remove "dir"',
          report={"status": "ok", "notable": [{"code": "delete_tree", "severity": "notice"}]},
          after=after({"dir__c_dot_txt": REMOVED, "dir__sub__d_dot_txt": REMOVED}))
    c.add("overwrote-existing", "Moving onto an existing file reports the overwrite.", fs, 'move "b.txt" to: "a.txt"',
          report={"status": "ok", "notable": [{"code": "overwrote_existing", "severity": "warning"}]},
          after=after({"b_dot_txt": REMOVED, "a_dot_txt": "bravo\n"}))
    c.add("file-emptied", "Emptying a file reports it.", fs, 'write "b.txt" "" mode: overwrite',
          report={"status": "ok", "notable": [{"code": "file_emptied"}, {"code": "overwrote_existing"}]},
          after=after({"b_dot_txt": ""}))
    big = {"big.txt": "x" * 2048 + "\n"}
    c.add("size-shrunk", "Shrinking a file by more than half reports it.", tree(big), 'write "big.txt" "x\\n" mode: overwrite',
          report={"status": "ok", "notable": [{"code": "size_shrunk"}, {"code": "overwrote_existing"}]},
          after=tree({"big.txt": "x\n"}))
    c.add("many-per-file", "Many replacements in one file are reported.", tree({"m.txt": "x" * 150 + "\n"}),
          'replace "x" "y" in: ["m.txt"]', report={"status": "ok", "notable": [{"code": "many_per_file", "count": 150}]},
          after=tree({"m.txt": "y" * 150 + "\n"}))
    c.add("nothing-matched", "A change without targets reports it and creates no operation.", fs,
          'find "." glob: "**/*.tmp" |> remove', report={"status": "ok", "notable": [{"code": "nothing_matched"}]})
    c.add("line-ending-preserved", "A change of a CRLF file reports the preserved line endings.", fs,
          'replace "two" "TWO" in: ["w.txt"]',
          report={"status": "ok", "notable": [{"code": "line_ending_preserved", "detail": "crlf"}]},
          after=after({"w_dot_txt": "one\r\nTWO\r\n"}))
    c.add("line-ending-changed", "A conversion of line endings is reported.", fs, 'convert "w.txt" line_ending: lf',
          report={"status": "ok", "notable": [{"code": "line_ending_changed"}]}, after=after({"w_dot_txt": "one\ntwo\n"}))
    c.add("binary-target", "Changing a binary file is reported.", tree({"b.bin": b"\x00a\x00"}),
          'replace "a" "b" in: ["b.bin"] binary: text',
          report={"status": "ok", "notable": [{"code": "binary_target"}]}, after=tree({"b.bin": b"\x00b\x00"}))
    for name, program in (
        ("write", 'write "a.txt" "x\\n" mode: overwrite'), ("replace", 'replace "a" "b" in: ["a.txt"]'),
        ("remove", 'remove "dir"'), ("move", 'move "a.txt" to: "z.txt"'), ("create", 'create "n" type: directory'),
        ("edit", 'edit "a.txt" changes: [{replace: "alpha", with: "A"}]'),
        ("set", 'set "c.json" key: "version" value: "2.0.0"'), ("convert", 'convert "w.txt" line_ending: lf'),
    ):
        c.add(f"preview-{name}", f"A preview of {name} reports the planned effect and changes nothing.", fs, program,
              options={"preview": True}, report={"status": "planned"})
    c.add("change-summary", "The report of a replacement counts files, replacements and lines.", fs,
          'replace "a" "A" in: ["."] glob: "**/*.txt"',
          report={"status": "ok", "summary": {"files_modified": BULK_FILES, "replacements": BULK_REPLACEMENTS}},
          after=after(CHANGES[-1][2]))
    c.add("change-diff", "The report of a replacement carries the difference.", fs, 'replace "beta" "BETA" in: ["a.txt"]',
          report={"status": "ok", "diff": [{"path": "${base}/a.txt", "lines": ["-beta", "+BETA"]}]},
          after=after({"a_dot_txt": "alpha\nBETA\n"}))
    return c


def collections() -> list[Collection]:
    """Returns every parity collection of the safety tools."""
    return [undo_scenarios(), journal_scenarios(), transaction_scenarios(), check_scenarios()]
