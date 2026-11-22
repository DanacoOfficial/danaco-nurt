---
command: code.call_hierarchy
group: code
summary: List the callers and callees of a function.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.call_hierarchy

## Synopsis

```text
code.call_hierarchy <path> [name: value]...
```

## Description

Lists the functions that call the function at a position of `path` (or
named by `symbol`), the functions it calls, or both.

The language server answers first. Under `source: auto` without a usable
server, syntax analysis approximates the hierarchy: callers are the
functions and methods that contain a reference to the name in code, callees
are the functions of the project called in the body of the definition.
Calls of members (`object.name(...)`) are not callees of the syntax
analysis. The result then carries a `semantic_fallback` note with the
cause.

Each `reference` record is one caller or callee. Its `text` holds the
direction (`<-` for a caller, `->` for a callee), the kind, the qualified
name and, for more than one call, the number of calls (`x3`).

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Source file. |
| `line` | `int` | none | Line of the function, counted from 1. |
| `column` | `int` | none | Column of the function, counted from 1. |
| `symbol` | `text` | none | Function name, optionally qualified as Container.name, instead of a position. |
| `direction` | `enum(incoming\|outgoing\|both)` | `both` | Callers, callees or both. |
| `language` | `text` | `"auto"` | Programming language; auto detects it from the file extension. |
| `root` | `path` | detected from the location of the file | Project root for the language server. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `reference` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `language_server` |
| Fallback | `builtin` |

## Examples

List the callers of a function.

```nurt example
code.call_hierarchy "/srv/app/src/shapes.py" symbol: "scale" direction: incoming
```

```text expected
<- function total x2
```

List the functions called by a method.

```nurt example
code.call_hierarchy "/srv/app/src/cart.ts" symbol: "Cart.checkout" direction: outgoing
```

List callers and callees of the function at another line.

```nurt example
code.call_hierarchy "/srv/app/src/main.go" line: 42
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | Neither `line` nor `symbol` was given. | Give `line` or `symbol`. |
| `E_INVALID_VALUE` | `direction` or `source` is not valid, or the file has fewer lines than `line`. | Check the parameters. |
| `E_SYMBOL_NOT_FOUND` | `symbol` has no definition in the file. | Check the name with `code.symbols`. |
| `E_SYMBOL_AMBIGUOUS` | `symbol` has several definitions in the file. | Qualify the name as `Container.name`. |
| `E_UNSUPPORTED_LANGUAGE` | Syntax analysis is needed and no grammar is available for the file. | Give `language`. |
| `E_EXTERNAL_MISSING` | `source: lsp` and no server is available for the language. | Install the server named by `fix`. |
| `E_EXTERNAL_FAILED` | `source: lsp` and the server failed. | See the message of the server. |
| `E_TIMEOUT` | `source: lsp` and the server gave no answer within `code.timeout`. | Raise `code.timeout`, or use `source: auto`. |

## Related

`code.references`, `code.definition`, `code.profile`.
