---
command: code.outline
group: code
summary: List the symbols of a file as an outline.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/code.outline.md -->
<!-- Responsibility:   Documents code.outline. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# code.outline

## Synopsis

```text
code.outline <path> [name: value]...
```

## Description

Lists the symbols of one source file in source order, one `symbol` record
per definition; a container comes before the symbols it contains. The
`container` field gives the qualified name of the enclosing symbol, so
`depth` 0 selects the top-level symbols, 1 adds their members, and so on.

A file of a language with a built-in grammar is outlined by syntax analysis.
The language is detected from a modeline, the file name, the extension and
the shebang line, in this order. A file of another language is outlined by
its language server when one is available.

Local variables declared inside functions are not part of an outline.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Source file. |
| `depth` | `int` | no limit | Maximum nesting depth of symbols. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `symbol` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Outline a file.

```nurt example
code.outline "/home/user/app/src/server.py"
```

List the top-level symbols only.

```nurt example
code.outline "/home/user/app/src/lib.rs" depth: 0
```

List the methods of one class.

```nurt example
code.outline "/home/user/app/src/Cart.java" |> filter container == "Cart" and kind == "method"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `path` was not given. | Give `path`. |
| `E_UNSUPPORTED_LANGUAGE` | The file has no built-in grammar and no language server is available. | Install the server for the language. |
| `E_INVALID_VALUE` | `depth` is negative. | Give a depth of 0 or more. |
| `E_LIMIT` | The file is larger than `code.file_bytes_max`. | Raise the limit. |
| `E_NOT_FOUND` | The file does not exist. | Check the path. |
| `E_EXTERNAL_FAILED` | The language server of a language without a grammar failed. | See the message of the server. |
| `E_TIMEOUT` | The language server of a language without a grammar did not answer. | Raise `code.timeout`. |

## Related

`code.symbols`, `read view: outline`, `code.languages`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
