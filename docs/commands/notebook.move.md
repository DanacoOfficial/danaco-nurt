---
command: notebook.move
group: notebook
summary: Move a cell to another position.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.move

## Synopsis

```text
notebook.move path from: int to: int
```

## Description

`notebook.move` moves the cell at index `from` so that it ends at index `to`;
the cells in between shift by one. Both indexes name existing cells. The
moved cell and every other cell keep their text; only the order changes.
Moving a cell to its own index changes nothing.

The `effect` record carries the counter `cells_moved`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Notebook file; position 1. |
| `from` | `int` | required | Current index of the cell, counted from 0. |
| `to` | `int` | required | New index of the cell, counted from 0. |

## Input and output

Input: none; `notebook.move` starts a pipeline.

Output: one `effect` record.

## Effect

Category `change`, effect `local`, reversibility `reversible`; no external
programs. The notebook is the only target. The plan reads the file, applies
the change and keeps the new content; the content is then replaced in one
write through the journal, which checks that the file still has the entity
tag it had when it was read (`E_STALE` otherwise). The `change` record
carries the sizes before and after, the numbers of added and removed lines
and the difference of the file, which is also what a preview shows. Undo
restores the previous content.

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

Move the last cell to the front:

```nurt example
notebook.move "/home/user/sales.ipynb" from: 2 to: 0
```

Swap the first two cells:

```nurt example
notebook.move "/home/user/sales.ipynb" from: 0 to: 1
```

Moving past the last cell fails with `E_OUT_OF_RANGE` and changes nothing:

```nurt example
notebook.move "/home/user/sales.ipynb" from: 0 to: 3
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path. |
| `E_PATH_RELATIVE` | `path` is relative and no base directory is set | Give an absolute path or set the `base` option. |
| `E_FORMAT` | the file is not valid JSON, not UTF-8, not nbformat 4, or has repeated cell identifiers | Repair the file, convert it to nbformat 4, or run `notebook.normalize` for repeated identifiers. |
| `E_LIMIT` | the file is larger than the text limit | Raise the limit or split the notebook. |
| `E_STALE` | the notebook changed after it was read | Read the notebook again and repeat the change. |
| `E_OUT_OF_RANGE` | `from` or `to` names no cell | Give indexes listed by `notebook.cells`; `expected` shows the valid range. |

## Related

`notebook.cells` lists the cells and their identifiers; `notebook.ops`
applies several changes in one write; `read` shows a notebook as `content`
with its cells. `help skill: "notebooks"` describes working with notebooks.
