---
command: notebook.export
group: notebook
summary: Export a notebook to a new script, Markdown or text file.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.export

## Synopsis

```text
notebook.export path to: path format: script|markdown|text
```

## Description

`notebook.export` writes the notebook `path` to the new file `to`:

| `format` | Content |
|---|---|
| `script` | A script in the language of the kernel in the percent format: each cell starts with a `%%` marker comment; Markdown and raw cells become comments, with the marker `[markdown]` or `[raw]`. |
| `markdown` | Markdown cells as written, code cells as fenced blocks in the language of the kernel, each followed by fenced `text` blocks with the text of its outputs; raw cells as written. |
| `text` | The sources of all cells. |

Cells are separated by an empty line, and the file ends with one line break;
a notebook without cells gives an empty file. The language comes from
`metadata.language_info.name`, otherwise from `metadata.kernelspec.language`;
the comment marker of a script is `#`, `//`, `--`, `%` or `;;` by language.
Images and other binary outputs are left out. The notebook itself is not
changed.

The `effect` record carries the counters `files_created` and
`cells_exported`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Notebook file; position 1. |
| `to` | `path` | required | File to create; it must not exist. |
| `format` | `enum(script\|markdown\|text)` | required | Format of the exported file. |

## Input and output

Input: none; `notebook.export` starts a pipeline.

Output: one `effect` record.

## Effect

Category `change`, effect `local`, reversibility `reversible`; no external
programs. The only target is the new file `to`, created through the journal;
the journal checks that it still does not exist. Undo removes the file.

## Examples

The examples use the notebook `/home/user/sales.ipynb` in nbformat 4.5 with
three cells: the Markdown cell `intro` (`# Sales`), the code cell `load`,
which reads `sales.csv` into `df` and has a stream output `12 rows`, and the
code cell `plot` with an image output.

Export the notebook as a Python script:

```nurt example
notebook.export "/home/user/sales.ipynb" to: "/home/user/sales.py" format: script
```

```text expected
# %% [markdown]
# # Sales
# Monthly totals.

# %%
import pandas as pd
df = pd.read_csv("sales.csv")
print(len(df), "rows")

# %%
df.plot(x="month", y="total")
```

Export the notebook as Markdown with the text of the outputs:

```nurt example
notebook.export "/home/user/sales.ipynb" to: "/home/user/sales.md" format: markdown
```

Export the sources of all cells as text:

```nurt example
notebook.export "/home/user/sales.ipynb" to: "/home/user/sales.txt" format: text
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path. |
| `E_PATH_RELATIVE` | `path` is relative and no base directory is set | Give an absolute path or set the `base` option. |
| `E_FORMAT` | the file is not valid JSON, not UTF-8, not nbformat 4, or has repeated cell identifiers | Repair the file, convert it to nbformat 4, or run `notebook.normalize` for repeated identifiers. |
| `E_LIMIT` | the file is larger than the text limit | Raise the limit or split the notebook. |
| `E_EXISTS` | `to` already exists | Export to a new file, or remove the existing one first. |
| `E_NOT_FOUND` | the directory of `to` does not exist | Create the directory first. |
| `E_UNKNOWN_VALUE` | `format` is unknown | Use `script`, `markdown` or `text`. |

## Related

`notebook.cells` lists the cells; `read` shows a notebook as `content` with
its cells. `help skill: "notebooks"` describes working with notebooks.
