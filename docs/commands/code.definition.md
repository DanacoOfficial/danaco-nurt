---
command: code.definition
group: code
summary: Find the definition of the symbol at a position or with a name.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.definition

## Synopsis

```text
code.definition <path> [line: int] [column: int] [symbol: text] [language: text] [root: path] [source: auto|lsp|syntax]
```

## Description

Finds the definition of the identifier at `line` and `column` of `path`, or
of the definition named by `symbol` in `path`. A `symbol` may be qualified by
its container (`Circle.area`); a name with several definitions in the file
fails with `E_SYMBOL_AMBIGUOUS` and lists the candidates. Without `column`
the first identifier of the line is used.

The language server of the file answers first. Under `source: auto` a
missing, failing or slow server, or an empty answer, gives way to syntax
analysis: the definitions of the identifier in the file and then in the
files of the project in the same language. The result then carries a
`semantic_fallback` note with the cause. A cancelled request is never
replaced. Under `source: lsp` the answer of the server is final, also when
it is empty.

The project root is the nearest directory above the file that holds a
project marker (`pyproject.toml`, `package.json`, `tsconfig.json`, `go.mod`,
`Cargo.toml`, `compile_commands.json`, `.git`), or the directory of the
file.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Source file. |
| `line` | `int` | none | Line of the symbol, counted from 1; required unless `symbol` is given. |
| `column` | `int` | first identifier of the line | Column of the symbol, counted from 1 in characters. |
| `symbol` | `text` | none | Symbol name, optionally qualified as `Container.name`, instead of a position. |
| `language` | `text` | `"auto"` | Programming language; `auto` detects it from the file extension. |
| `root` | `path` | detected from the location of the file | Project root for the language server. |
| `source` | `enum(auto\|lsp\|syntax)` | `auto` | Language server, syntax analysis, or the server when available. |

## Input and output

| Form | Input | Output |
|---|---|---|
| source | none | `symbol` |

| Field | Type | Description |
|---|---|---|
| `name` | `text` | Name of the definition. |
| `kind` | `enum` | Kind of the symbol (`function`, `method`, `class`, ...). |
| `path` | `path` | File of the definition. |
| `line`, `column` | `int` | Position of the name, from 1. |
| `end_line`, `end_column` | `int` | End of the definition and of the name. |
| `container` | `text` | Qualified name of the enclosing symbol. |
| `signature` | `text` | Declaration line, up to 240 characters. |
| `text` | `text` | Source line of the name. |
| `language` | `text` | Language of the file. |
| `source` | `enum` | `lsp` or `syntax`. |
| `etag` | `text` | Entity tag of the file when it was read. |

## Effect

Category `source`, effect `read`, reversibility `read`. The language server
of the language is started on first use and stays in the server pool (see
`code.languages`).

## Examples

Find the definition of the identifier at line 12, column 8.

```nurt example
code.definition "/srv/app/src/main.py" line: 12 column: 8
```

Find a method by its qualified name.

```nurt example
code.definition "/srv/app/src/shapes.py" symbol: "Circle.area"
```

Require the answer of the language server.

```nurt example
code.definition "/srv/app/src/index.ts" line: 40 column: 15 source: lsp
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | Neither `line` nor `symbol` was given. | Give `line` or `symbol`. |
| `E_INVALID_VALUE` | The file has fewer lines than `line`, or `source` or `root` is not valid. | Check the position and the parameters. |
| `E_SYMBOL_NOT_FOUND` | `symbol` has no definition in the file. | Check the name with `code.symbols`, or give `line` and `column`. |
| `E_SYMBOL_AMBIGUOUS` | `symbol` has several definitions in the file; the samples list them. | Qualify the name as `Container.name`, or give `line` and `column`. |
| `E_UNSUPPORTED_LANGUAGE` | Syntax analysis is needed and no grammar is available for the file. | Give `language`. |
| `E_EXTERNAL_MISSING` | `source: lsp` and no server is available for the language. | Install the server named by `fix`, or allow it in `code.servers_allow`. |
| `E_EXTERNAL_FAILED` | `source: lsp` and the server failed. | See the message, which ends with the last line the server wrote to standard error. |
| `E_TIMEOUT` | `source: lsp` and the server gave no answer within `code.timeout`. | Raise `code.timeout`, or use `source: auto`. |
| `E_CANCELLED` | The request was cancelled. | Run the command again. |

## Related

`code.references`, `code.type_definition`, `code.symbols`, `code.languages`.
