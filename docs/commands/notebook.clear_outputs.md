---
command: notebook.clear_outputs
group: notebook
summary: Clear the outputs of notebook cells.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.clear_outputs

## Synopsis

```text
notebook.clear_outputs <path>
```

## Description

`notebook.clear_outputs` sets `outputs` to an empty list and
`execution_count` to `null` in the selected code cells: by default every
cell, otherwise the cells whose index lies in the range `index` or the cells
with the identifiers `id`. Markdown and raw cells have no outputs and are not
changed; a code cell that already has no outputs and no counter keeps its
text. A notebook without outputs is not written.

The `effect` record carries the counter `cells_cleared`, the number of cells
that changed.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Notebook file; position 1. |
| `index` | `range` | `all` | Cells to clear by index, counted from 0. Excludes `id`. |
| `id` | `list<text>` | none | Cells to clear by identifier. Excludes `index`. |

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

Clear the outputs of every cell:

```nurt example
notebook.clear_outputs "/home/user/sales.ipynb"
```

Clear the outputs of the cell at index 1:

```nurt example
notebook.clear_outputs "/home/user/sales.ipynb" index: 1
```

Clear the image output of the cell `plot`:

```nurt example
notebook.clear_outputs "/home/user/sales.ipynb" id: ["plot"]
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path. |
| `E_PATH_RELATIVE` | `path` is relative and no base directory is set | Give an absolute path or set the `base` option. |
| `E_FORMAT` | the file is not valid JSON, not UTF-8, not nbformat 4, or has repeated cell identifiers | Repair the file, convert it to nbformat 4, or run `notebook.normalize` for repeated identifiers. |
| `E_LIMIT` | the file is larger than the text limit | Raise the limit or split the notebook. |
| `E_STALE` | the notebook changed after it was read | Read the notebook again and repeat the change. |
| `E_OUT_OF_RANGE` | a bound of `index` names no cell | Give indexes listed by `notebook.cells`. |
| `E_ARG` | no cell has an identifier of `id` | Use identifiers listed by `notebook.cells`. |
| `E_PARAM_CONFLICT` | both `index` and `id` are given | Give one of them. |

## Related

`notebook.cells` lists the cells and their identifiers; `notebook.ops`
applies several changes in one write; `read` shows a notebook as `content`
with its cells. `help skill: "notebooks"` describes working with notebooks.
