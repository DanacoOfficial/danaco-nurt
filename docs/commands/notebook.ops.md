---
command: notebook.ops
group: notebook
summary: Apply a list of cell operations in one write.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/notebook.ops.md -->
<!-- Responsibility:   Documents notebook.ops. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# notebook.ops

## Synopsis

```text
notebook.ops <path> changes: <list<map<any>>>
```

## Description

`notebook.ops` applies the operations of `changes` in order, each to the
result of the previous ones, and writes the notebook once. When an operation
fails, nothing is changed: the error names the operation in
`details.change`, counted from 0.

Each operation is a map with the member `action` and the parameters of the
corresponding command:

| `action` | Members | As |
|---|---|---|
| `insert` | `cell_type`, `source`, `index` or `after_id` | `notebook.insert` |
| `replace` | `index` or `id`, `source`, `cell_type`, `find`, `with`, `whole_word`, `reset_outputs` | `notebook.replace` |
| `move` | `index` or `id` of the cell, `to` | `notebook.move` |
| `copy` | `index` or `id` of the cell, `to` (directly after the original when missing) | `notebook.copy` |
| `remove` | `index` (an index or a range such as `"1..3"`) or `id` (one or a list) | `notebook.remove` |
| `clear_outputs` | `index` or `id`; every cell when both are missing | `notebook.clear_outputs` |

A member that does not apply to the action is an error. `find` may be text or
a regular expression.

The `effect` record carries the counters `operations`, `cells_inserted`,
`cells_changed`, `cells_moved`, `cells_copied`, `cells_removed`,
`cells_cleared` and `replacements`, each when it is not zero, and in `after`
a list with the `change`, `index` and `id` of every inserted or copied cell.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Notebook file. |
| `changes` | `list<map<any>>` | required | Operations applied together; all succeed or none. |

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

Insert a note after the title and remove the plot, in one write:

```nurt example
notebook.ops "/home/user/sales.ipynb" changes: [{action: "insert", after_id: "intro", cell_type: "markdown", source: "Data source: sales.csv"}, {action: "remove", id: "plot"}]
```

Rename a variable in one cell and clear every output:

```nurt example
notebook.ops "/home/user/sales.ipynb" changes: [{action: "replace", id: "load", find: "df", with: "sales", whole_word: true}, {action: "clear_outputs"}]
```

A failing operation leaves the notebook unchanged; here the second operation
names a cell that does not exist:

```nurt example
notebook.ops "/home/user/sales.ipynb" changes: [{action: "remove", index: 0}, {action: "move", index: 9, to: 0}]
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path. |
| `E_PATH_RELATIVE` | `path` is relative and no base directory is set | Give an absolute path or set the `base` option. |
| `E_FORMAT` | the file is not valid JSON, not UTF-8, not nbformat 4, or has repeated cell identifiers | Repair the file, convert it to nbformat 4, or run `notebook.normalize` for repeated identifiers. |
| `E_LIMIT` | the file is larger than the text limit | Raise the limit or split the notebook. |
| `E_STALE` | the notebook changed after it was read | Read the notebook again and repeat the change. |
| `E_ARG` | an operation has an unknown action, a member that does not apply, a missing or mistyped member, or names an unknown identifier; or `changes` is empty | Correct the operation named in `details.change`. |
| `E_OUT_OF_RANGE` | an operation names an index outside the notebook | Correct the operation named in `details.change`; indexes refer to the result of the earlier operations. |
| `E_NO_MATCH` | the `find` of an operation occurs in no selected cell | Check the text or the cell of that operation. |
| `E_REGEX` | a regular expression or replacement template of an operation is invalid | Correct the pattern or the template. |
| `E_TYPE` | `changes` is not a list | Give a list of maps. |

## Related

`notebook.cells` lists the cells and their identifiers; `notebook.ops`
applies several changes in one write; `read` shows a notebook as `content`
with its cells. `help skill: "notebooks"` describes working with notebooks.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
