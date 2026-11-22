---
command: code.hover
group: code
summary: Show the signature and documentation of the symbol at a position.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.hover

## Synopsis

```text
code.hover <path> [name: value]...
```

## Description

Shows the signature and the documentation of the identifier at a position
of `path`, or of the definition named by `symbol`.

The answer of the language server is shortened: the signature is the first
code block of the answer, with a signature written over several lines
joined into one line, and the documentation is the first sentence of the
remaining text. With `verbosity: detailed` in the request options the
documentation is the whole answer of the server.

Under `source: auto` without a usable server, or when the server has no
information, the definition is found by syntax analysis; the signature is
its declaration line and the documentation is the first sentence of the
docstring below the name or of the comment lines above the definition. The
result then carries a `semantic_fallback` note with the cause. Under
`source: lsp` a server without information gives no record.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Source file. |
| `line` | `int` | none | Line of the symbol, counted from 1; required unless symbol is given. |
| `column` | `int` | none | Column of the symbol, counted from 1; required unless symbol is given. |
| `symbol` | `text` | none | Symbol name, optionally qualified as Container.name, instead of a position. |
| `language` | `text` | `"auto"` | Programming language; auto detects it from the file extension. |
| `root` | `path` | detected from the location of the file | Project root for the language server. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `hover` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `language_server` |
| Fallback | `builtin` |

## Examples

Show the signature of the function called at line 5.

```nurt example
code.hover "/srv/app/src/main.py" line: 5 column: 12
```

```text expected
signature: def scale(value) -> int
```

Show the signature and the documentation of a class.

```nurt example
code.hover "/srv/app/src/shapes.py" symbol: "Circle" |> select signature documentation
```

Read the documentation of a symbol by name.

```nurt example
code.hover "/srv/app/src/shapes.py" symbol: "scale"
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

`code.definition`, `code.symbols`, `code.outline`.
