---
command: code.actions
group: code
summary: List the code actions a language server offers for a range of lines.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/code.actions.md -->
<!-- Responsibility:   Documents code.actions. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# code.actions

## Synopsis

```text
code.actions <path> line: <int> [name: value]...
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
| `end_line` | `int` | none | Last line of the range. |
| `column` | `int` | none | Column within the first line, counted from 1. |
| `language` | `text` | `"auto"` | Programming language; auto detects it from the file extension. |
| `root` | `path` | detected from the location of the file | Project root for the language server. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `code_action` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `language_server` |
| Fallback | `builtin` |

## Examples

List the actions for one line.

```nurt example
code.actions "/srv/app/src/main.py" line: 14
```

List the refactorings offered for a block of lines.

```nurt example
code.actions "/srv/app/src/main.ts" line: 20 end_line: 34 |> filter kind == "refactor.extract"
```

List the quick fixes at an error.

```nurt example
code.actions "/srv/app/src/lib.rs" line: 52 column: 17 |> filter kind == "quickfix" |> select title
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

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
