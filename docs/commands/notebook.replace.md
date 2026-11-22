---
command: notebook.replace
group: notebook
summary: Replace the source or type of a cell, or text within cell sources.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.replace

## Synopsis

```text
notebook.replace <path> [name: value]...
```

## Description

The first form changes one cell, named by `index` or `id`: `source` replaces
its whole source, `cell_type` changes its type and `reset_outputs` clears its
outputs and execution counter. At least one of them is required.

The second form replaces every match of `find` with `with` within the source
of the named cell or, without `index` and `id`, within the sources of all
cells. `whole_word` matches only where the text is neither preceded nor
followed by a letter, a digit or `_`, so `df` does not match within `df2`.
With a regular expression, `with` is a template: `$1` or `${1}` inserts a
capture group by number, `$0` the whole match and `$$` a `$`. With
`reset_outputs`, the cells in which text was replaced lose their outputs. At
least one match is required.

`source` and `find` exclude each other. The source keeps the form stored in
the cell, a list of lines or a single string. A change of `cell_type` adds
`outputs` and `execution_count` when the cell becomes a code cell and removes
them, together with `attachments` of code cells, otherwise. Outputs stay
unless `reset_outputs` is set.

When both `index` and `id` are given, they must name the same cell.

The `effect` record carries the counters `cells_changed` and, with `find`,
`replacements`; the `change` record carries `replacements` too.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Notebook file. |
| `index` | `int` | none | Cell to change by index, counted from 0. |
| `id` | `text` | none | Cell to change by identifier. |
| `source` | `text` | none | New source of the cell. |
| `cell_type` | `enum(code\|markdown\|raw)` | none | New type of the cell. |
| `find` | `text \| regex` | none | Text or pattern to replace within cell sources. |
| `with` | `text` | none | Replacement for the text matched by find. |
| `whole_word` | `bool` | `false` | Match find only as a whole word or identifier. |
| `reset_outputs` | `bool` | `false` | Clear the outputs of changed cells. |

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

Replace the source of the last cell:

```nurt example
notebook.replace "/home/user/sales.ipynb" index: 2 source: "df.plot(x=\"month\", y=\"total\", kind=\"bar\")"
```

Rename the variable `df` in every cell:

```nurt example
notebook.replace "/home/user/sales.ipynb" find: "df" with: "sales" whole_word: true
```

Rewrite a call with a regular expression in the cell `load`:

```nurt example
notebook.replace "/home/user/sales.ipynb" id: "load" find: /read_(\w+)/ with: "load_$1"
```

Clear the outputs of the cell `load`:

```nurt example
notebook.replace "/home/user/sales.ipynb" id: "load" reset_outputs: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path. |
| `E_PATH_RELATIVE` | `path` is relative and no base directory is set | Give an absolute path or set the `base` option. |
| `E_FORMAT` | the file is not valid JSON, not UTF-8, not nbformat 4, or has repeated cell identifiers | Repair the file, convert it to nbformat 4, or run `notebook.normalize` for repeated identifiers. |
| `E_LIMIT` | the file is larger than the text limit | Raise the limit or split the notebook. |
| `E_STALE` | the notebook changed after it was read | Read the notebook again and repeat the change. |
| `E_OUT_OF_RANGE` | `index` names no cell | Give an index listed by `notebook.cells`. |
| `E_ARG` | no cell has the identifier, `index` and `id` name different cells, or the arguments are incomplete or inconsistent | Follow the message: give one cell, `source` or `find` with `with`. |
| `E_NO_MATCH` | `find` occurs in no selected cell | Check the text or the cell selection. |
| `E_REGEX` | `find` is not a valid regular expression, or `with` references a group that does not exist | Correct the pattern or the template; write `$$` for a literal `$`. |

## Related

`notebook.cells` lists the cells and their identifiers; `notebook.ops`
applies several changes in one write; `read` shows a notebook as `content`
with its cells. `help skill: "notebooks"` describes working with notebooks.
