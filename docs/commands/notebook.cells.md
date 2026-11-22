---
command: notebook.cells
group: notebook
summary: List notebook cells, as an outline or with full content.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.cells

## Synopsis

```text
notebook.cells path [index: range] [id: list<text>] [cell_type: code|markdown|raw] [outputs: none|summary|text|full] [find: text|regex] [scope: source|outputs] [view: outline|full]
```

## Description

`notebook.cells` produces one `cell` record for each selected cell of the
notebook `path`, in the order of the cells. Every record carries `index`
(from 0), `id` (nbformat 4.5 and later), `cell_type`, `source`,
`execution_count` of code cells and `etag`, the entity tag of the file.

Without `index`, `id` and `find`, every cell is listed as an outline: `source`
is the first non-blank line and `metadata` is left out. With any of them,
the selected cells are listed in full, with the whole source and the
metadata. A `view` given explicitly overrides that choice.

`index` and `id` select cells together: a cell is listed when its index lies
in the range or its identifier is listed. The start of `index` must name an
existing cell; the end may lie past the last cell. `cell_type` keeps only
cells of that type. `find` keeps only cells in which the text or regular
expression occurs, in the source or, with `scope: outputs`, in the text of
the outputs. Regular expressions use the dialect of `search`.

`outputs` sets the detail of the outputs of code cells: `summary` adds
`outputs_summary` with the number and kinds of outputs, for example
`1 output: stream stdout`; `text` also lists each output reduced to its type,
stream name and text, without images and other binary data; `full` lists the
outputs as stored.

A notebook without cells produces no records.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Notebook file; position 1. |
| `index` | `range` | all cells | Cells to return by index, counted from 0, for example `2` or `1..4`. |
| `id` | `list<text>` | none | Cells to return by identifier. |
| `cell_type` | `enum(code\|markdown\|raw)` | any type | Only cells of this type. |
| `outputs` | `enum(none\|summary\|text\|full)` | `none` | Detail of cell outputs returned. |
| `find` | `text \| regex` | none | Only cells containing this text or pattern. |
| `scope` | `enum(source\|outputs)` | `source` | Part of the cell that `find` searches. |
| `view` | `enum(outline\|full)` | `outline` | Outline or full content; without `view`, selected cells are listed in full. |

## Input and output

Input: none; `notebook.cells` starts a pipeline.

Output: `cell` records, ordered by `index`.

## Effect

Category `source`, effect `read`, reversibility `read`; no external programs.
`notebook.cells` changes nothing and writes no journal entry.

## Examples

The examples use the notebook `/home/user/sales.ipynb` in nbformat 4.5 with
three cells: the Markdown cell `intro` (`# Sales`), the code cell `load`,
which reads `sales.csv` into `df` and has a stream output `12 rows`, and the
code cell `plot` with an image output.

List the outline of every cell:

```nurt example
notebook.cells "/home/user/sales.ipynb"
```

```text expected
0  markdown  intro  # Sales
1  code      load   import pandas as pd
2  code      plot   df.plot(x="month", y="total")
```

Show one cell in full with the text of its outputs:

```nurt example
notebook.cells "/home/user/sales.ipynb" index: 1 outputs: text
```

List the cells whose source calls a `read_` function:

```nurt example
notebook.cells "/home/user/sales.ipynb" find: /read_\w+/
```

Summarize the outputs of the code cells:

```nurt example
notebook.cells "/home/user/sales.ipynb" cell_type: code outputs: summary
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path. |
| `E_PATH_RELATIVE` | `path` is relative and no base directory is set | Give an absolute path or set the `base` option. |
| `E_FORMAT` | the file is not valid JSON, not UTF-8, not nbformat 4, or has repeated cell identifiers | Repair the file, convert it to nbformat 4, or run `notebook.normalize` for repeated identifiers. |
| `E_LIMIT` | the file is larger than the text limit | Raise the limit or split the notebook. |
| `E_OUT_OF_RANGE` | the start of `index` lies past the last cell | List the outline first to see the valid indexes. |
| `E_ARG` | no cell has an identifier of `id` | Use an identifier from the outline; the message names the closest ones. |
| `E_REGEX` | `find` is not a valid regular expression | Correct the pattern at the reported position. |
| `E_UNKNOWN_VALUE` | a value of `cell_type`, `outputs`, `scope` or `view` is unknown | Use one of the listed values. |

## Related

`read` shows a notebook as `content` with its cells; `notebook.replace`,
`notebook.insert` and `notebook.ops` change cells. `help skill: "notebooks"`
describes working with notebooks.
