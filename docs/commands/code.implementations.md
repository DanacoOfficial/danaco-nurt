---
command: code.implementations
group: code
summary: Find implementations of an interface or types derived from the symbol.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.implementations

## Synopsis

```text
code.implementations <path> [line: int] [column: int] [symbol: text] [language: text] [root: path] [source: auto|lsp|syntax]
```

## Description

Finds the implementations of the interface, trait or abstract method at a
position of `path`, or of the definition named by `symbol`, and the types
derived from a class.

The language server answers first. Under `source: auto` without a usable
server, or with an empty answer, syntax analysis lists the classes,
structures, interfaces and enumerations of the project whose declaration
names the identifier after `extends`, `implements` or `for`, after `impl`,
or in a list of base types. The result then carries a `semantic_fallback`
note with the cause. Syntax analysis does not follow aliases or imports
under another name.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Source file. |
| `line` | `int` | none | Line of the symbol, counted from 1; required unless `symbol` is given. |
| `column` | `int` | first identifier of the line | Column of the symbol, counted from 1. |
| `symbol` | `text` | none | Symbol name, optionally qualified as `Container.name`, instead of a position. |
| `language` | `text` | `"auto"` | Programming language; `auto` detects it from the file extension. |
| `root` | `path` | detected from the location of the file | Project root for the language server. |
| `source` | `enum(auto\|lsp\|syntax)` | `auto` | Language server, syntax analysis, or the server when available. |

## Input and output

| Form | Input | Output |
|---|---|---|
| source | none | `symbol` |

The fields of the `symbol` record are described in `code.definition`.

## Effect

Category `source`, effect `read`, reversibility `read`.

## Examples

List the classes that implement an interface.

```nurt example
code.implementations "/srv/app/src/repo.ts" symbol: "Repository"
```

List the implementations of a trait method at a position.

```nurt example
code.implementations "/srv/app/src/store.rs" line: 18 column: 8
```

List the derived classes found by syntax analysis with their files.

```nurt example
code.implementations "/srv/app/src/shapes.py" symbol: "Shape" source: syntax |> select name, path, line
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | Neither `line` nor `symbol` was given. | Give `line` or `symbol`. |
| `E_INVALID_VALUE` | The file has fewer lines than `line`, or `source` is not valid. | Check the parameters. |
| `E_SYMBOL_NOT_FOUND` | `symbol` has no definition in the file. | Check the name with `code.symbols`. |
| `E_SYMBOL_AMBIGUOUS` | `symbol` has several definitions in the file. | Qualify the name as `Container.name`. |
| `E_UNSUPPORTED_LANGUAGE` | Syntax analysis is needed and no grammar is available for the file. | Give `language`. |
| `E_EXTERNAL_MISSING` | `source: lsp` and no server is available for the language. | Install the server named by `fix`. |
| `E_EXTERNAL_FAILED` | `source: lsp` and the server failed. | See the message of the server. |
| `E_TIMEOUT` | `source: lsp` and the server gave no answer within `code.timeout`. | Raise `code.timeout`, or use `source: auto`. |

## Related

`code.definition`, `code.type_definition`, `code.symbols`.
