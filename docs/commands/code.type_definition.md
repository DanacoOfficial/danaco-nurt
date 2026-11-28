---
command: code.type_definition
group: code
summary: Find the definition of the type of the symbol.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/code.type_definition.md -->
<!-- Responsibility:   Documents code.type_definition. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# code.type_definition

## Synopsis

```text
code.type_definition <path> [name: value]...
```

## Description

Finds the definition of the type of the variable, parameter, field or
expression at a position of `path`, or of the definition named by `symbol`.

The type of a symbol is known only to a language server, so the command has
no syntax analysis: without a server for the language it fails with
`E_EXTERNAL_MISSING`, and a failure of the server is the error of the
command. `source: syntax` fails with `E_EXTERNAL_MISSING` as well.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Source file. |
| `line` | `int` | none | Line of the symbol, counted from 1. |
| `column` | `int` | none | Column of the symbol, counted from 1. |
| `symbol` | `text` | none | Symbol name, optionally qualified as Container.name, instead of a position. |
| `language` | `text` | `"auto"` | Programming language; auto detects it from the file extension. |
| `root` | `path` | detected from the location of the file | Project root for the language server. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `symbol` |

The fields of the `symbol` record are described in `code.definition`;
`source` is always `lsp`.

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `language_server` |
| Fallback | `builtin` |

## Examples

Find the type of the variable at a position.

```nurt example
code.type_definition "/srv/app/src/main.ts" line: 22 column: 9
```

Find the type of a field named by its container.

```nurt example
code.type_definition "/srv/app/src/model.py" symbol: "Order.customer"
```

Show the file and line of the type.

```nurt example
code.type_definition "/srv/app/src/main.ts" line: 22 column: 9 |> select name path line
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | Neither `line` nor `symbol` was given. | Give `line` or `symbol`. |
| `E_INVALID_VALUE` | The file has fewer lines than `line`, or `source` is not valid. | Check the parameters. |
| `E_SYMBOL_NOT_FOUND` | `symbol` has no definition in the file. | Check the name with `code.symbols`. |
| `E_SYMBOL_AMBIGUOUS` | `symbol` has several definitions in the file. | Qualify the name as `Container.name`. |
| `E_EXTERNAL_MISSING` | No server is available for the language. | Install the server named by `fix`, or allow it in `code.servers_allow`. |
| `E_EXTERNAL_FAILED` | The server failed. | See the message of the server. |
| `E_TIMEOUT` | The server gave no answer within `code.timeout`. | Raise `code.timeout`. |

## Related

`code.definition`, `code.hover`, `code.languages`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
