---
command: notebook.copy
group: notebook
summary: Copy a cell to another position.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.copy

## Synopsis

```text
notebook.copy <path> from: <int> to: <int>
```

## Description

`notebook.copy` inserts a copy of the cell at index `from` at index `to`,
which may be any position from 0 to the number of cells. The copy repeats the
text of the original exactly, outputs and metadata included, except for a new
identifier when the notebook uses identifiers.

The `effect` record carries the counter `cells_copied` and, in `after`, the
`index` and `id` of the copy.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Notebook file; position 1. |
| `from` | `int` | required | Index of the cell to copy, counted from 0. |
| `to` | `int` | required | Index of the copy, from 0 to the number of cells. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

Cells and members the change does not touch keep their bytes: key order,
indentation, number notation, escapes and the layout of the file are
preserved, so a difference of the file shows only the changed cells. A call
that leaves the notebook as it was writes nothing and creates no journal
entry; its `effect` record has `targets: 0`.

## Examples

The examples use the notebook `/home/user/sales.ipynb` in nbformat 4.5 with
three cells: the Markdown cell `intro` (`# Sales`), the code cell `load`,
which reads `sales.csv` into `df` and has a stream output `12 rows`, and the
code cell `plot` with an image output.

Copy the cell `load` directly after itself:

```nurt example
notebook.copy "/home/user/sales.ipynb" from: 1 to: 2
```

Repeat the first cell of a notebook in nbformat 4.4, whose cells have no
identifiers:

```nurt example
notebook.copy "/home/user/sales.ipynb" from: 0 to: 0
```

Copying a cell that does not exist fails with `E_OUT_OF_RANGE`:

```nurt example
notebook.copy "/home/user/sales.ipynb" from: 9 to: 0
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path. |
| `E_PATH_RELATIVE` | `path` is relative and no base directory is set | Give an absolute path or set the `base` option. |
| `E_FORMAT` | the file is not valid JSON, not UTF-8, not nbformat 4, or has repeated cell identifiers | Repair the file, convert it to nbformat 4, or run `notebook.normalize` for repeated identifiers. |
| `E_LIMIT` | the file is larger than the text limit | Raise the limit or split the notebook. |
| `E_STALE` | the notebook changed after it was read | Read the notebook again and repeat the change. |
| `E_OUT_OF_RANGE` | `from` names no cell, or `to` is greater than the number of cells | Give indexes listed by `notebook.cells`. |

## Related

`notebook.cells` lists the cells and their identifiers; `notebook.ops`
applies several changes in one write; `read` shows a notebook as `content`
with its cells. `help skill: "notebooks"` describes working with notebooks.
