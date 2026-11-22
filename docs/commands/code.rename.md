---
command: code.rename
group: code
summary: Rename a symbol and its references across files in one operation.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.rename

## Synopsis

```text
symbol |> code.rename <to> [name: value]...
code.rename <to> path: <path> [name: value]...
```

## Description

Renames one symbol and every reference to it. The source form addresses the
symbol by `path` with `line` and `column`, or by `symbol`, a name or a
qualified name such as `Parser.parse`. Without `column`, the first identifier
of `line` is used. The stage form renames each input `symbol` record.

The edits come from the language server of the language (`source: lsp`) or
from syntax analysis (`source: syntax`), which replaces the whole identifier
in code of the files of the same language family (TypeScript and JavaScript
form one family, C and C++ another). `source: auto` uses the server when one
is available and syntax analysis otherwise; a server that fails, times out
or returns no edits is reported as `semantic_fallback`. Occurrences in
comments and string literals are renamed only with `include_comments` and
`include_strings`.

The plan is checked before anything is written:

- an edit outside the scope is not applied and is reported as
  `outside_project` (outside `root` and `paths`) or `rename_incomplete`
  (another file under `scope: file`);
- a changed file that already uses the new name in code fails with
  `E_RENAME_CONFLICT`;
- syntax analysis that finds more than one definition of the name fails
  with `E_SYMBOL_AMBIGUOUS`, because every definition would be renamed;
- server edits that do more than replace the old name are listed in the
  `alarms` field of the plan summary.

The result holds one `rename_edit` record per edit and a `map` record with the
plan summary: `plan` (identifier), `from`, `to`, `source`, `server`,
`confidence` (`semantic` for server edits, `identifier` for syntax
analysis), `root`, `files`, `total`, `alarms` and `leftovers`.

Execution writes every file in one journal operation, together with the file
creations, renames and deletions requested by the server. A file changed
after it was read fails with `E_STALE` before anything is written. After the
change, syntax errors and language server errors of every written file are
compared with the errors before the change; a new error restores every file
and fails with `E_ROLLED_BACK`. With `check_references`, occurrences of the
old name left in code, comments, strings, documentation, configuration and
file names are reported as `references_left`; occurrences left in code of the
same language family are also reported as `rename_incomplete`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `to` | `text` | required | New name of the symbol. |
| `path` | `path` | none | Source file of the symbol in the source form. |
| `line` | `int` | none | Line of the symbol in the source form, counted from 1. |
| `column` | `int` | none | Column of the symbol in the source form, counted from 1. |
| `symbol` | `text` | none | Symbol name, optionally qualified as Container.name, instead of a position. |
| `scope` | `enum(project\|file)` | `project` | Rename in the whole project or only in the file. |
| `paths` | `list<path>` | the project | Files and directories to rename in. |
| `include_comments` | `bool` | `false` | Also rename occurrences in comments. |
| `include_strings` | `bool` | `false` | Also rename occurrences in string literals. |
| `source` | `enum(auto\|lsp\|syntax)` | `auto` | Language server, syntax analysis with confidence, or the server when available. |
| `check_references` | `bool` | `true` | Report references to the old name left after the change. |
| `language` | `text` | `"auto"` | Programming language; auto detects it from the file extension. |
| `root` | `path` | detected from the location of the file | Project root for the language server. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `stage` | `symbol` | `effect` |
| `source` | `none` | `effect` |

The `effect` record has `kind: "code.rename"`, the changed files in
`changes`, the counters `files_modified`, `files_created`, `files_deleted`,
`files_renamed` and `replacements` in `summary`, and the codes of the
notable effects in `notable`.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `language_server` |
| Reversibility conditions | All changed files are restored together by one undo. |
| Fallback | `builtin` |

## Examples

Rename the function `total` defined in `math.py`.

```nurt example
code.rename "sum_all" path: "/srv/app/src/math.py" symbol: "total"
```

Rename the symbol at line 12, column 5, also in comments.

```nurt example
code.rename "parse_header" path: "/srv/app/src/reader.py" line: 12 column: 5 include_comments: true
```

Rename a method found by name, with syntax analysis limited to one
directory.

```nurt example
code.symbols "loadConfig" path: ["/srv/app/src"] kind: method match: exact |> code.rename "readConfig" source: syntax paths: ["/srv/app/src"]
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | `to` is not an identifier or equals the current name. | Give another name. |
| `E_MISSING_PARAM` | Neither `symbol` nor `line` was given. | Give `symbol`, or `line` and `column`. |
| `E_SYMBOL_NOT_FOUND` | No identifier at the position, no definition of `symbol`, or the server found nothing to rename. | Check the position or the name. |
| `E_SYMBOL_AMBIGUOUS` | `symbol` has several definitions in the file, or syntax analysis found several definitions of the name; `candidates` lists one call for each definition. | Give `line` and `column`, narrow `paths`, or use `source: lsp`. |
| `E_RENAME_CONFLICT` | The new name is already used in a changed file. | Choose another name. |
| `E_UNSUPPORTED_LANGUAGE` | No grammar for the file and no server. | Give `language` or install the server. |
| `E_EXTERNAL_MISSING` | `source: lsp` and no server for the language. | Install the server or use `source: syntax`. |
| `E_EXTERNAL_FAILED` | `source: lsp` and the server failed. | See the message of the server. |
| `E_STALE` | A file changed after the plan. | Run the rename again. |
| `E_ROLLED_BACK` | The rename introduced a new error; every file was restored. | Review the reported error. |
| `E_LIMIT` | More than 500 files would change, or a file exceeds `code.file_bytes_max`. | Narrow `paths` or raise the limit. |

## Related

`code.references`, `code.symbols`, `code.diagnostics`, `undo`,
`help skill: "refactoring"`.
