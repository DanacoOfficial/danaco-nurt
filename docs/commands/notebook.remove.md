---
command: notebook.remove
group: notebook
summary: Remove cells from a notebook.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.remove

## Synopsis

```text
notebook.remove <path>
```

## Description

`notebook.remove` removes the cells whose index lies in the range `index`, or
the cells with the identifiers `id`. Both bounds of a range must name
existing cells. The other cells keep their text.

The `effect` record carries the counter `cells_removed`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Notebook file; position 1. |
| `index` | `range` | none | Cells to remove by index, counted from 0, for example `2` or `0..1`. Excludes `id`. |
| `id` | `text \| list<text>` | none | Cells to remove by identifier. Excludes `index`. |

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

Remove the last cell:

```nurt example
notebook.remove "/home/user/sales.ipynb" index: 2
```

Remove two cells by identifier:

```nurt example
notebook.remove "/home/user/sales.ipynb" id: ["intro", "plot"]
```

Remove the first two cells:

```nurt example
notebook.remove "/home/user/sales.ipynb" index: 0..1
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path. |
| `E_PATH_RELATIVE` | `path` is relative and no base directory is set | Give an absolute path or set the `base` option. |
| `E_FORMAT` | the file is not valid JSON, not UTF-8, not nbformat 4, or has repeated cell identifiers | Repair the file, convert it to nbformat 4, or run `notebook.normalize` for repeated identifiers. |
| `E_LIMIT` | the file is larger than the text limit | Raise the limit or split the notebook. |
| `E_STALE` | the notebook changed after it was read | Read the notebook again and repeat the change. |
| `E_MISSING_PARAM` | neither `index` nor `id` is given | Name the cells to remove. |
| `E_OUT_OF_RANGE` | a bound of `index` names no cell | Give indexes listed by `notebook.cells`. |
| `E_ARG` | no cell has an identifier of `id` | Use identifiers listed by `notebook.cells`; the message names the closest ones. |
| `E_PARAM_CONFLICT` | both `index` and `id` are given | Give one of them. |

## Related

`notebook.cells` lists the cells and their identifiers; `notebook.ops`
applies several changes in one write; `read` shows a notebook as `content`
with its cells. `help skill: "notebooks"` describes working with notebooks.
