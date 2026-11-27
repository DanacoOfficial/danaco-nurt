---
command: code.references
group: code
summary: Find references to the symbol at a position, with a name or from the input.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/code.references.md -->
<!-- Responsibility:   Documents code.references. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# code.references

## Synopsis

```text
code.references <path> [name: value]...
symbol |> code.references [name: value]...
```

## Description

Finds the references to the identifier at a position of `path`, or to the
definition named by `symbol`. In the stage form every input `symbol` record
is a target, at the position of its name.

The language server answers first. Under `source: auto` without a usable
server, syntax analysis finds the occurrences of the name in code in the
files of the project in the same language; occurrences in comments and
strings are not references. The result then carries a `semantic_fallback`
note with the cause.

Every reference has a role:

| Role | Meaning |
|---|---|
| `definition` | The definition of the symbol. |
| `import` | An import or include line. |
| `test` | A use in a test file (`tests/`, `test_*.py`, `*_test.go`, `*.spec.ts` and similar). |
| `reference` | Any other use. |

Records are free of repetitions and ordered by path, line and column; the
number of records of each role is the count of that role.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Source file in the source form. |
| `line` | `int` | none | Line of the symbol, counted from 1. |
| `column` | `int` | none | Column of the symbol, counted from 1. |
| `symbol` | `text` | none | Symbol name, optionally qualified as Container.name, instead of a position. |
| `include_definition` | `bool` | `true` | Include the definition among the references. |
| `scope` | `enum(project\|file)` | `project` | Search the whole project or only the file. |
| `language` | `text` | `"auto"` | Programming language; auto detects it from the file extension. |
| `root` | `path` | detected from the location of the file | Project root for the language server. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `source` | `none` | `reference` |
| `stage` | `symbol` | `reference` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `language_server` |
| Fallback | `builtin` |

## Examples

Find every reference to a function.

```nurt example
code.references "/srv/app/src/shapes.py" symbol: "scale"
```

List the uses of a symbol in test files.

```nurt example
code.references "/srv/app/src/shapes.py" symbol: "scale" include_definition: false |> filter role == "test"
```

Find the references to every exported class of a directory.

```nurt example
code.symbols path: ["/srv/app/src"] kind: class exported_only: true |> code.references scope: project
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | Neither `line` nor `symbol` was given in the source form. | Give `line` or `symbol`. |
| `E_INVALID_VALUE` | `scope`, `source` or the position is not valid. | Check the parameters. |
| `E_TYPE` | An input record is not a `symbol` record. | Pipe the output of `code.symbols` or `code.definition`. |
| `E_SYMBOL_NOT_FOUND` | `symbol` has no definition in the file. | Check the name with `code.symbols`. |
| `E_SYMBOL_AMBIGUOUS` | `symbol` has several definitions in the file. | Qualify the name as `Container.name`, or give `line` and `column`. |
| `E_UNSUPPORTED_LANGUAGE` | Syntax analysis is needed and no grammar is available for the file. | Give `language`. |
| `E_EXTERNAL_MISSING` | `source: lsp` and no server is available for the language. | Install the server named by `fix`. |
| `E_EXTERNAL_FAILED` | `source: lsp` and the server failed. | See the message of the server. |
| `E_TIMEOUT` | `source: lsp` and the server gave no answer within `code.timeout`. | Raise `code.timeout`, or use `source: auto`. |

## Related

`code.definition`, `code.call_hierarchy`, `code.rename`, `code.symbols`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
