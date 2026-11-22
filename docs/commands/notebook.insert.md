---
command: notebook.insert
group: notebook
summary: Insert a new cell into a notebook.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.insert

## Synopsis

```text
notebook.insert <path> cell_type: <enum(code|markdown|raw)> source: <text>
```

## Description

`notebook.insert` adds one cell to the notebook `path`: at `index`, directly
after the cell with the identifier `after_id`, or after the last cell when
neither is given. The cell at `index` and the cells after it move down by
one.

The new cell has empty `metadata`; a code cell also has `execution_count:
null` and no outputs. Its `source` is stored in the form the other cells use,
a list of lines or a single string. When the notebook is in nbformat 4.5 or
later, or its cells already have identifiers, the new cell gets a new
identifier of eight hexadecimal digits that no other cell has.

The `effect` record carries the counter `cells_inserted` and, in `after`,
the `index` and `id` of the new cell.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Notebook file. |
| `cell_type` | `enum(code\|markdown\|raw)` | required | Type of the new cell. |
| `source` | `text` | required | Source of the new cell. |
| `index` | `int` | the end of the notebook | Position of the new cell, counted from 0. |
| `after_id` | `text` | none | Insert after the cell with this identifier. |

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

Append a Markdown cell:

```nurt example
notebook.insert "/home/user/sales.ipynb" cell_type: markdown source: "## Totals"
```

Insert a code cell before the cell `load`, at index 1:

```nurt example
notebook.insert "/home/user/sales.ipynb" index: 1 cell_type: code source: "import numpy as np"
```

Insert a code cell after the cell `load`:

```nurt example
notebook.insert "/home/user/sales.ipynb" after_id: "load" cell_type: code source: "df.describe()"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path. |
| `E_PATH_RELATIVE` | `path` is relative and no base directory is set | Give an absolute path or set the `base` option. |
| `E_FORMAT` | the file is not valid JSON, not UTF-8, not nbformat 4, or has repeated cell identifiers | Repair the file, convert it to nbformat 4, or run `notebook.normalize` for repeated identifiers. |
| `E_LIMIT` | the file is larger than the text limit | Raise the limit or split the notebook. |
| `E_STALE` | the notebook changed after it was read | Read the notebook again and repeat the change. |
| `E_OUT_OF_RANGE` | `index` is negative or greater than the number of cells | Give an index from 0 to the number of cells. |
| `E_ARG` | no cell has the identifier `after_id` | Use an identifier listed by `notebook.cells`. |
| `E_PARAM_CONFLICT` | both `index` and `after_id` are given | Give one of them. |
| `E_UNKNOWN_VALUE` | `cell_type` is unknown | Use `code`, `markdown` or `raw`. |

## Related

`notebook.cells` lists the cells and their identifiers; `notebook.ops`
applies several changes in one write; `read` shows a notebook as `content`
with its cells. `help skill: "notebooks"` describes working with notebooks.
