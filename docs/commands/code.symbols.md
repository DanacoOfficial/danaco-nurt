---
command: code.symbols
group: code
summary: Find symbol definitions by name across a repository.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.symbols

## Synopsis

```text
code.symbols [name] [path: <list<path>>] [glob: <glob>] [kind: <kind>] [match: <mode>]
             [language: <text>] [source: auto|lsp|syntax] [with_signature: <bool>]
             [with_references: <bool>] [exported_only: <bool>] [exclude: <list<glob>>]
             [exclude_preset: <list<preset>>]
```

## Description

Lists the definitions whose names match `name` in the given files and
directories, one `symbol` record per definition.

With `source: auto` and a name, the language server of the project answers
first. When no server answers, without a name, and with `exported_only`, the
symbols come from syntax analysis with the built-in grammars, and a server
failure is reported as `semantic_fallback`.

Syntax analysis of a directory uses a symbol index kept in the state
directory. A later search reads only the files whose size or modification
time changed. The listing applies ignore files and skips version control,
dependency and cache directories. `.git`, `node_modules`, `__pycache__`,
`venv` and `.venv` are always skipped, as are files larger than
`code.file_bytes_max`. The index holds no more than `code.index_max_files`
files. When the limit is reached, the result is incomplete and the report
carries `source_unavailable`.

A directory search covers programming languages. Data and markup formats
(JSON, YAML, TOML, Markdown, HTML, CSS, XML) are searched when `language`
names them or when `path` names a file. Files without an extension are
searched when their shebang line or modeline names a language.

Names are ranked from 6 (same text) to 1 (the letters of the query appear in
order). Records are ordered by rank, then by path and position. Without a
name every symbol is listed in path and source order.

| Rank | Condition | Modes |
|---|---|---|
| 6 | same text | all |
| 5 | same text regardless of case; for `regex`, equal to the pattern | all |
| 4 | starts with the query, regardless of case | `prefix`, `substring`, `fuzzy` |
| 3 | contains the query, regardless of case | `substring`, `fuzzy` |
| 2 | the words of the query start words of the name in order; any `regex` match | `fuzzy`, `regex` |
| 1 | the letters and digits of the query appear in order | `fuzzy` |

Words are split at camelCase boundaries and at characters other than
letters and digits, so `getUserById`, `get_user_by_id` and `GetUserByID`
share the words `get`, `user`, `by`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text \| regex \| list<text>` | all symbols | Symbol name, pattern or names; a list matches any of them. |
| `path` | `list<path>` | base directory | Files or directories to search. |
| `glob` | `glob` | `"**"` | Only files whose path relative to the searched directory matches. |
| `kind` | `enum` | `any` | `function`, `method`, `class`, `struct`, `interface`, `enum`, `variable`, `constant`, `field`, `module`, `type`, `macro` or `any`. |
| `match` | `enum` | `fuzzy` | `exact`, `prefix`, `substring`, `fuzzy` or `regex`. |
| `language` | `text` | `"auto"` | Language identifier or alias (`py`, `ts`, `c#`); `auto` detects it per file. |
| `source` | `enum` | `auto` | `lsp`, `syntax`, or `auto` for the server when available. |
| `with_signature` | `bool` | `false` | Fill `signature`. |
| `with_references` | `bool` | `false` | Fill `references`. |
| `exported_only` | `bool` | `false` | Only symbols visible outside their module. |
| `exclude` | `list<glob>` | `[]` | Paths to skip, in gitignore syntax. |
| `exclude_preset` | `list<enum>` | `[]` | `vcs`, `deps`, `build`, `cache`, `tests` (test files by the conventions of each language). |

## Input and output

| Form | Input | Output |
|---|---|---|
| source | none | `symbol` |

| Field | Type | Description |
|---|---|---|
| `name` | `text` | Name as written. |
| `kind` | `enum` | Kind of the symbol. |
| `path` | `path` | File. |
| `line`, `column` | `int` | Position of the name, from 1; columns count characters. |
| `end_line`, `end_column` | `int` | End of the whole definition. |
| `language` | `text` | Language of the file. |
| `source` | `enum` | `lsp` or `syntax`. |
| `etag` | `text` | Content digest of the file, `sha256:` followed by 64 hexadecimal digits. |
| `container` | `text` | Qualified name of the enclosing symbol (`Outer.Inner`). |
| `signature` | `text` | Declaration without the body, with `with_signature`. |
| `exported` | `bool` | Visibility by the rules of the language; absent where the language has none. |
| `references` | `int` | With `with_references`: references reported by the server, or occurrences of the name in code outside definitions in files of the same language. |

Visibility rules: Python, a name not starting with `_`; Go, a capitalized
name; Rust, a visibility modifier; JavaScript and TypeScript, an `export` at
top level; C and C++, not `static`; Java and C#, `public`; Kotlin, not
`private`, `protected` or `internal`; Swift, `public` or `open`; PHP, not
`private` or `protected`.

## Effect

Category `source`, effect `read`, reversibility `read`. With a language
server, the server of the project is started when it is not running. The
symbol index is written to the state directory.

## Examples

Find a function by an abbreviation of its words.

```nurt example
code.symbols "getUser" kind: function
```

List the exported classes of a directory without test files.

```nurt example
code.symbols path: ["src"] kind: class exported_only: true exclude_preset: [tests]
```

Find the handlers with their signatures and reference counts from syntax analysis.

```nurt example
code.symbols /Handler$/ match: regex source: syntax with_signature: true with_references: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | Unknown `match`, `kind` or `exclude_preset` value, or an invalid glob. | Use a value listed under Parameters. |
| `E_REGEX` | The regular expression of `name` is not valid. | Correct the pattern. |
| `E_UNSUPPORTED_LANGUAGE` | `language` names no known language, or the language of a file given in `path` is not known. | Check the names with `code.languages`. |
| `E_PARAM_CONFLICT` | `exported_only` with `source: lsp`: servers do not report visibility. | Use `source: syntax` or `auto`. |
| `E_EXTERNAL_MISSING` | `source: lsp` without an available language server. | Install the server named by `fix`. |
| `E_MISSING_PARAM` | No `path` and no base directory. | Give `path`. |
| `E_LIMIT` | A file given in `path` is larger than `code.file_bytes_max`. | Raise the limit or give a directory. |
| `E_NOT_FOUND` | A path does not exist. | Check the path. |

## Related

`code.outline`, `code.definition`, `code.references`, `code.languages`,
configuration keys `code.index_max_files`, `code.file_bytes_max`.
