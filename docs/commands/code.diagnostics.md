---
command: code.diagnostics
group: code
summary: Report errors and warnings in source files.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.diagnostics

## Synopsis

```text
code.diagnostics <path>... [name: value]...
```

## Description

Reports the errors and warnings of the given files. A directory stands for
the source files and structured documents below it, without `.git`,
`node_modules`, `__pycache__`, `venv` and `.venv`, up to
`code.index_max_files` files. With `changed: true` only the files changed in
the repository are checked: changes in the working tree and the index and
untracked files, limited to the given paths when there are any.

Each file is checked by its language server. A server that supports pull
diagnostics is asked; for the others the client opens the file and waits
until the server has published nothing for `code.diagnostics_quiet`
(1500 ms). Under `source: auto` a file without a usable server is checked
by syntax analysis: the grammar of its language or, for structured
documents (JSON, YAML, TOML and the other document formats), the parser of
the format. Each distinct cause of a fallback is reported once as a
`semantic_fallback` note.

Records are free of repetitions and ordered by severity, path, line and
column. For more than five files the command reports its progress.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `list<path>` | none | Files or directories to check. |
| `changed` | `bool` | `false` | Only files changed in the repository. |
| `severity` | `enum(error\|warning\|info\|hint)` | `hint` | Lowest severity reported. |
| `source` | `enum(auto\|lsp\|syntax)` | `auto` | Language server, syntax analysis, or the server when available. |
| `language` | `text` | `"auto"` | Programming language; auto detects it from file extensions. |
| `root` | `path` | detected from the location of the file | Project root for the language server. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `diagnostic` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `language_server` |
| Fallback | `builtin` |

## Examples

Check one file.

```nurt example
code.diagnostics "/srv/app/src/main.py"
```

Report the errors of a directory only.

```nurt example
code.diagnostics "/srv/app/src" severity: error
```

Check the files changed in the repository and list the files with errors.

```nurt example
code.diagnostics changed: true |> filter severity == "error" |> select path line message
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | No path was given and `changed` is `false`. | Give a path or `changed: true`. |
| `E_INVALID_VALUE` | `severity` or `source` is not valid, or the start path of `changed: true` is not in a repository. | Check the parameters. |
| `E_ARG` | `changed: true` without a path and without a base directory. | Give a path inside the repository. |
| `E_LIMIT` | A file is larger than `code.file_bytes_max`. | Raise the limit or leave out the file. |
| `E_ENCODING` | A file is not UTF-8 text. | Convert the file to UTF-8. |
| `E_EXTERNAL_MISSING` | `source: lsp` and no server is available, or `changed: true` and repository queries are not available. | Install the server named by `fix`, or give the paths. |
| `E_EXTERNAL_FAILED` | `source: lsp` and the server failed. | See the message of the server. |
| `E_TIMEOUT` | `source: lsp` and the server gave no answer within `code.timeout`. | Raise `code.timeout`, or use `source: auto`. |

## Related

`code.languages`, `validate`, configuration keys `code.diagnostics_quiet`,
`code.timeout`.
