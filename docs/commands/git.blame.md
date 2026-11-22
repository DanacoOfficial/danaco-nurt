---
command: git.blame
group: git
summary: Show the commit and author that last changed each line of a file.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.blame

## Synopsis

```text
git.blame <path> [name: value]...
```

## Description

Annotates each line of a file with the commit that last changed it, in line
order. The history is followed as `git blame` follows it by default: through
renames of the whole file, comparing lines with the Myers algorithm. Lines
moved within the file or copied from another file are attributed to the
commit that moved or copied them unless `find_moves` or `find_copies` is
set. Identities are mapped through `.mailmap`. Root commits are reported as
boundaries.

Without `revision`, the file of the working tree is annotated after the
built-in conversions (`core.autocrlf`, `eol`, `text`); filter programs are
not run. Lines that differ from `HEAD` belong to no commit: their commit is
the null identifier and their author `Not Committed Yet`. A file that is
staged but not in `HEAD` has only such lines. With `revision`, the file of
that commit is annotated.

`find_moves` (`git blame -M`) attributes a block of lines that a commit moved
within the file to the commit that wrote it, when the block holds at least
20 alphanumeric characters. `find_copies` (`git blame -C`) implies
`find_moves` and also looks for blocks of at least 40 alphanumeric characters
in the other files that the same commit changed. Lines of a commit with
several parents are searched in each parent in turn.

Line breaks, including a carriage return before the line feed, are not part
of `text`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | File in a repository. |
| `lines` | `range` | none | Lines to annotate, counted from 1; an end beyond the file is cut to its last line. |
| `revision` | `text` | `"HEAD"` | Commit whose file is annotated. |
| `find_moves` | `bool` | `false` | Attribute lines moved within the file to the commit that wrote them. |
| `find_copies` | `bool` | `false` | As `find_moves`, and also lines copied from other files changed by the same commit. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `blame_line` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Annotate a file of the working tree.

```nurt example
git.blame "/home/user/repo/src/main.rs"
```

```text expected
blame_line no: 1 short_id: 9b07d3e author: "Ann Lee" text: "use std::io;"
blame_line no: 2 short_id: 0000000 author: "Not Committed Yet" text: "use std::fs;"
```

Annotate lines 40 to 60 as they were in a release.

```nurt example
git.blame "/home/user/repo/src/parser.rs" lines: 40..60 revision: "v1.4.0"
```

Count the annotated lines per author.

```nurt example
git.blame "/home/user/repo/src/parser.rs" revision: "HEAD" |> group author |> sort count order: desc
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `path` is not inside a repository. | Give a file inside a working tree |
| `E_GIT_REVISION` | `revision` is unknown, ambiguous or names no commit. | List branches with `git.branch` |
| `E_NOT_FOUND` | The file does not exist at `revision`, or is neither in `HEAD` nor staged. | Find the file with `git.log` or stage it |
| `E_INVALID_VALUE` | `path` names a directory, a submodule or the repository root, or `lines` starts before line 1 or after the last line. | Give a file and a range inside it |
| `E_IO` | An object of the repository cannot be read. | Check the repository with `git.status` |

## Related

`git.log`, `git.show`, `git.diff`.
