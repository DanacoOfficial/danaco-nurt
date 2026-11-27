---
command: notebook.normalize
group: notebook
summary: Add missing cell identifiers and nbformat 4 fields.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/notebook.normalize.md -->
<!-- Responsibility:   Documents notebook.normalize. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# notebook.normalize

## Synopsis

```text
notebook.normalize <path>
```

## Description

`notebook.normalize` brings the notebook `path` to nbformat 4.5:

- `nbformat_minor` becomes 5 when it is lower, and a missing notebook
  `metadata` is added;
- every cell without a valid identifier, or with an identifier an earlier
  cell already has, gets a new identifier of eight hexadecimal digits;
- missing `metadata` and `source` are added to cells, and missing `outputs`
  and `execution_count` to code cells;
- members nbformat does not allow for the type of a cell are removed:
  `outputs` and `execution_count` of Markdown and raw cells, `attachments`
  of code cells.

Cells that need none of these changes keep their text. A second call changes
nothing and writes nothing. The other notebook commands never add
identifiers or members to cells they do not change.

The `effect` record carries the counter `cells_changed`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Notebook file. |

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

Add identifiers to a notebook in nbformat 4.4, whose cells have none:

```nurt example
notebook.normalize "/home/user/sales.ipynb"
```

Give a new identifier to the second of two cells that share the identifier
`load`:

```nurt example
notebook.normalize "/home/user/sales.ipynb"
```

Add `outputs` and `execution_count` to a code cell that lacks them:

```nurt example
notebook.normalize "/home/user/sales.ipynb"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path. |
| `E_PATH_RELATIVE` | `path` is relative and no base directory is set | Give an absolute path or set the `base` option. |
| `E_FORMAT` | the file is not valid JSON, not UTF-8, not nbformat 4, or has repeated cell identifiers | Repair the file, convert it to nbformat 4, or run `notebook.normalize` for repeated identifiers. |
| `E_LIMIT` | the file is larger than the text limit | Raise the limit or split the notebook. |
| `E_STALE` | the notebook changed after it was read | Read the notebook again and repeat the change. |

## Related

`notebook.cells` lists the cells and their identifiers; `notebook.ops`
applies several changes in one write; `read` shows a notebook as `content`
with its cells. `help skill: "notebooks"` describes working with notebooks.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
