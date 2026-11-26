---
command: git.diff
group: git
summary: Show differences between revisions, the index and the working tree.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.diff

## Synopsis

```text
git.diff <repo> [name: value]...
```

## Description

Compares two sides of a repository and returns the differences as hunks in
the unified format of `git diff`. The sides are chosen as follows.

| Parameters | Old side | New side |
|---|---|---|
| none | index | working tree |
| `staged` | `HEAD` | index |
| `from` | revision `from` | working tree |
| `from`, `staged` | revision `from` | index |
| `from`, `to` | revision `from` | revision `to` |

`path` selects files and directories literally, relative to the repository
root. Files with conflict stages are left out. Renames and copies are
detected as configured by `diff.renames` (on by default) with a similarity
threshold of 50 percent.

Lines are compared with the algorithm set by `diff.algorithm` (`myers` by
default, `minimal`, `histogram`; `patience` is computed with `histogram` and
noted as `semantic_fallback`) and the indentation heuristic
(`diff.indentHeuristic`, on by default). Changes at most twice the context
apart, plus `diff.interHunkContext`, share a hunk; a request for the canonical
layout (no `from` and `to`, the default context of 3 lines, without
`function_context`, `ignore_whitespace` and `symbol`) ignores
`diff.interHunkContext`, so its hunks and their identifiers do not depend on
the configuration. The text after `@@` is the
nearest function line above the hunk, found by the pattern of the `diff`
driver of the path (`diff.<driver>.xfuncname`, the built-in patterns for
`rust`, `python`, `golang`, `java`, `cpp`, `markdown` and `bash`, or the
default rule of `git`).

A file is binary when the attribute `-diff` or `binary` is set, when its
driver sets `diff.<driver>.binary`, or when its first 8000 bytes contain a NUL
byte; a binary file yields one record without lines. A change of the mode
only, or a rename without a change of content, yields one record without
lines. A missing line feed at the end of a file is written as the line
`\ No newline at end of file`.

Text conversion programs (`diff.<driver>.textconv`) and filter drivers are not
run: the stored content is compared and the report notes
`filter_not_applied`.

### Hunk identifiers

Without `from`, `to`, `symbol`, `function_context` and `ignore_whitespace` and
with the default context of 3 lines, every record of a file that is not
renamed carries an identifier `path#number` and a `fingerprint`. The number
counts the hunks of the file from 1 in the difference of the working tree
against the index, or of the index against `HEAD` with `staged`. The
fingerprint is the first six hexadecimal digits of the SHA-256 of the hunk
lines, with their prefixes, joined by line feeds. `git.stage hunks:` and
`git.restore hunks:` accept the identifier as `path#number`,
`path#number:fingerprint` or with rows `path#number@2-4,7`; a fingerprint that
no longer matches is rejected.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `from` | `text` | the index or HEAD | Old revision. |
| `to` | `text` | the working tree | New revision. |
| `staged` | `bool` | `false` | Compare the index with HEAD. |
| `path` | `list<path>` | none | Only these files or directories. |
| `context` | `int` | `3` | Lines of context around each change. |
| `stat` | `bool` | `false` | Only the numbers of added and removed lines per file. |
| `function_context` | `bool` | `false` | Extend each change to the whole enclosing function. |
| `symbol` | `text` | none | Only changes within this code element on the new side. |
| `word_diff` | `bool` | `false` | Show changes word by word. |
| `ignore_whitespace` | `bool` | `false` | Ignore changes in whitespace. |
| `ignore_moved` | `bool` | `false` | Hide blocks of lines that were only moved. |
| `exclude` | `list<glob>` | `[]` | Paths to leave out; the report counts the excluded files. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `hunk` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Show the unstaged changes of one file.

```nurt example
git.diff "/home/user/repo" path: ["src/main.rs"]
```

```text expected
hunk path: src/main.rs id: src/main.rs#1 old_start: 10 old_len: 7 new_start: 10 new_len: 8
```

Summarize what a branch changed since the main branch.

```nurt example
git.diff "/home/user/repo" from: "main" to: "feature" stat: true
```

Show the staged changes with whole functions, ignoring white space.

```nurt example
git.diff "/home/user/repo" staged: true function_context: true ignore_whitespace: true
```

Show the changes of one function since the previous commit.

```nurt example
git.diff "/home/user/repo" from: "HEAD~1" path: ["src/parser.rs"] symbol: "parse_expr"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `to` without `from`, or `symbol` without a file. | Add the missing parameter |
| `E_ARITY` | `symbol` with more than one file. | Give one file in `path` |
| `E_ARG` | `staged` together with `to` (problem `param_conflict`). | Remove one of them |
| `E_INVALID_VALUE` | `context` outside 0 to 200, or an invalid word pattern. | Use a value in range |
| `E_GIT_REVISION` | A revision does not exist or does not name a commit or tree. | Check the name with `git.log "/home/user/repo"` |
| `E_GIT_NOT_REPOSITORY` | The path is not inside a repository, a path lies outside it, or the working tree is needed in a bare repository. | Give a path inside the working tree |
| `E_SYMBOL_NOT_FOUND` | `symbol` is not defined in the new version of the file. | Check the name of the element |
| `E_STAGE_FAILED` | An object, the index or a file cannot be read (problem `read_failed`). | Restore the missing objects of the repository |

## Related

`git.status`, `git.stage`, `git.show`, `git.review`, `git.checkpoint`.
