---
command: edit
group: core
summary: Apply a list of changes to one file, all or none.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/edit.md -->
<!-- Responsibility:   Documents edit. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# edit

## Synopsis

```text
edit <path> changes: <list<map<any>>> [name: value]...
```

## Description

`edit` applies a list of changes to one text file as one operation: either
every change is applied or the file is not changed. A change is a map of one
of three kinds:

| Change | Keys | Effect |
|---|---|---|
| replacement | `replace` (text or regular expression), `with`, and optionally `occurrence`, `whole_word`, `whitespace`, `on_missing`, `expect` | replaces the matches of the pattern, as `replace` does |
| insertion | `insert` (text), and optionally `line` and `position` | inserts lines before or after `line`, or at the `start` or `end` of the file, as `insert` does |
| deletion | `delete_lines` (range or line number) | deletes the lines of the range |

Every change is located in the file as it was before the edit: line numbers
and matches of a change refer to the original text, never to the text left
by the changes listed before it. Insertions at the same place keep the order
of the list, and an insertion at the line where a deletion starts comes
before it, so `{delete_lines: a..b}` together with
`{insert: …, line: a, position: before}` replaces lines `a` to `b`. Changes
whose ranges overlap are rejected before anything is written.

`whitespace` and `on_missing` of the command apply to the replacements
without their own value. `expect` of the command applies to every
replacement without its own `expect`. A replacement that matches nothing
with `on_missing: skip` is reported as `nothing_matched` with its number. A
list of changes that leaves the text as it was writes nothing and is
reported as `unchanged`.

The file keeps its encoding, byte order mark and line endings; inserted and
replaced lines take the line endings of the file.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | File to change. |
| `changes` | `list<map<any>>` | required | Changes: replace and with, insert with line and position, or delete_lines. |
| `expect` | `range` | none | Expected number of replacements of each change. |
| `if_match` | `text` | none | Expected etag of the file. |
| `whitespace` | `enum(exact\|trailing\|indent\|normalize)` | `normalize` | Tolerance of whitespace differences. |
| `on_missing` | `enum(skip\|error)` | `skip` | Behaviour when a change matches nothing. |
| `validate` | `enum(off\|warn\|reject)` | `reject` | Roll back changes that add a new syntax error to a known format. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

## Examples

Change a value and add a line after it:

```nurt example
edit "/home/user/app/config.toml" changes: [{replace: "debug = true", with: "debug = false"}, {insert: "log = \"info\"", line: 2, position: after}]
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/config.toml", "type": "file", "content_utf8": "[server]\ndebug = true\nport = 8080\n" }
] }
```

Replace lines 2 to 4 with one line:

```nurt example
edit "/home/user/app/util.py" changes: [{delete_lines: 2..4}, {insert: "    return a + b", line: 2, position: before}]
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/util.py", "type": "file", "content_utf8": "def add(a, b):\n    total = a + b\n    print(total)\n    return total\n" }
] }
```

Raise the patch version when exactly one version line exists:

```nurt example
edit "/home/user/app/Cargo.toml" changes: [{replace: /version = "(\d+)\.(\d+)\.\d+"/, with: "version = \"$1.${2}.3\"", occurrence: 1}] expect: 1
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/Cargo.toml", "type": "file", "content_utf8": "version = \"1.4.2\"\nedition = \"2024\"\n" }
] }
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | a change has no or several kinds, an unknown key, or overlaps another change | correct the change or merge the overlapping changes |
| `E_OUT_OF_RANGE` | a line lies outside the file, or a replacement with `on_missing: error` names an occurrence above the number of its matches | give a line from 1 to the number of lines, or a smaller `occurrence` |
| `E_MISSING_PARAM` | a replacement has no `with`, or an insertion before or after a line has no `line` | add the key, or use `position: start` or `end` |
| `E_ASSERT` | the number of matches of a replacement is outside `expect` | extend the pattern, set `occurrence` or change `expect` |
| `E_NO_MATCH` | a replacement matches nothing and `on_missing` is `error` | read the current text and give an existing fragment |
| `E_REGEX` | a regular expression or a template is not valid | correct the pattern or the template |
| `E_STALE` | the file changed since it was read, or does not have the `if_match` tag | read the file again and repeat the edit |
| `E_ENCODING` | the encoding of the file cannot represent the new text | convert the file with `convert` first |
| `E_BINARY` | the file is binary | edit text files only |
| `E_ROLLED_BACK` | the edit added a syntax error and `validate` is `reject` | correct the changes or pass `validate: warn` |

## Related

`replace`, `insert`, `write`, `patch`, `read`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
