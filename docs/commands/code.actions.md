---
command: code.actions
group: code
summary: List the code actions a language server offers for a range of lines.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.actions

## Synopsis

```text
code.actions <path> line: int [end_line: int] [column: int] [language: text] [root: path] [source: auto|lsp]
```

## Description

Lists the code actions (quick fixes, refactorings, source actions) that the
language server offers for the lines `line` to `end_line` of `path`. The
range starts at `column` of the first line, or at its start, and ends at
the end of the last line. The diagnostics the server published for the
range are sent as the context of the request.

The command lists actions and applies none. Code actions are offered only
by a language server: without a server for the language the command fails
with `E_EXTERNAL_MISSING`, and a failure of the server is the error of the
command.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Source file. |
| `line` | `int` | required | First line of the range, counted from 1. |
| `end_line` | `int` | value of `line` | Last line of the range. |
| `column` | `int` | start of the line | Column within the first line, counted from 1. |
| `language` | `text` | `"auto"` | Programming language; `auto` detects it from the file extension. |
| `root` | `path` | detected from the location of the file | Project root for the language server. |
| `source` | `enum(auto\|lsp)` | `auto` | Both values ask the language server. |

## Input and output

| Form | Input | Output |
|---|---|---|
| source | none | `code_action` |

| Field | Type | Description |
|---|---|---|
| `path` | `path` | File. |
| `line`, `end_line` | `int` | Range asked for. |
| `title` | `text` | Title of the action, as offered by the server. |
| `kind` | `text` | Protocol kind (`quickfix`, `refactor.extract`, `source.organizeImports`, ...), or `command` for an action that only runs a server command. |
| `source` | `enum` | Always `lsp`. |

## Effect

Category `source`, effect `read`, reversibility `read`. No action is
applied; changes of a file go through `replace` or `code.rename`.

## Examples

List the actions for one line.

```nurt example
code.actions "/srv/app/src/main.py" line: 14
```

List the refactorings offered for a block of lines.

```nurt example
code.actions "/srv/app/src/main.ts" line: 20 end_line: 34 |> where kind == "refactor.extract"
```

List the quick fixes at an error.

```nurt example
code.actions "/srv/app/src/lib.rs" line: 52 column: 17 |> where kind == "quickfix" |> select title
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `line` was not given. | Give `line`. |
| `E_INVALID_VALUE` | `end_line` is before `line`, or the file has fewer lines than the range. | Give a range inside the file. |
| `E_EXTERNAL_MISSING` | No server is available for the language. | Install the server named by `fix`, or allow it in `code.servers_allow`. |
| `E_EXTERNAL_FAILED` | The server failed. | See the message of the server. |
| `E_TIMEOUT` | The server gave no answer within `code.timeout`. | Raise `code.timeout`. |

## Related

`code.diagnostics`, `code.rename`, `replace`.
