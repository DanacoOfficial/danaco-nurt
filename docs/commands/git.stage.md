---
command: git.stage
group: git
summary: List, stage or unstage changes down to selected hunks and lines.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/git.stage.md -->
<!-- Responsibility:   Documents git.stage. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# git.stage

## Synopsis

```text
git.stage <repo> action: <enum(list|add|unstage)> [name: value]...
```

## Description

`action: list` reports the changes of the working tree against the index, or
of the index against `HEAD` with `staged: true`, as `hunk` records. Each hunk
has an identifier `<path>#<number>:<fingerprint>` that `add` and `unstage`
accept; the fingerprint makes a later call fail with `E_STALE` when the hunk
changed in between.

`action: add` copies the working tree state of the selection into the index,
as `git add` does: changed and new files are staged, files deleted from the
working tree are removed from the index, and a staged conflicted file is
resolved, with its stages kept as resolve-undo data. A directory, and `.` for
the whole working tree, stands for the files below it: the entries of the
index and the untracked files that are not ignored. Files matching a pattern
of `exclude` are left out and counted. An ignored file named explicitly in
`paths` is staged, which `git add` refuses without `-f`; the report carries an
`ignored` note that lists such files.

`action: unstage` copies the `HEAD` state of the selection into the index, as
`git restore --staged` does: an entry returns to its content and mode in
`HEAD`, an entry missing from `HEAD` is removed, and the working tree does not
change.

`hunks` selects parts of files. `<path>#<number>` selects a whole hunk and
`<path>#<number>@<rows>` selected rows of it, where rows are numbers and
ranges such as `3-5,8` counted over the lines of the hunk. For `add` an
unselected added line is dropped and an unselected removed line stays; for
`unstage` the selection is taken out of the index content. The result equals
`git apply --cached` with the patch reduced to the selection. A new file can
be staged partially; files without a final line feed and with CRLF line
endings keep their bytes.

The index is written under `index.lock` through the journal; new blobs are
added to the object database. Index entries written by the command carry no
file status, as entries written by `git apply --cached`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `action` | `enum(list\|add\|unstage)` | required | List changes as hunks, add them to the index, or remove them from it. |
| `paths` | `list<path>` | none | Files or directories to act on. |
| `hunks` | `list<text>` | none | Hunks by identifier, such as file#2 or file#2@3-5,8 for selected lines. |
| `staged` | `bool` | none | With list: show staged instead of unstaged changes. |
| `exclude` | `list<glob>` | `[]` | Paths to leave out of add; the report counts the excluded files. |
| `intent_to_add` | `bool` | `false` | With add: record the paths in the index without their content. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect`, `hunk` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

| Action | Effect | Reversibility | Note |
|---|---|---|---|
| `list` | `read` | `read` |  |

## Examples

List the unstaged hunks of one file.

```nurt example
git.stage "/home/user/repo" action: list paths: ["src/main.rs"]
```

```text expected
hunk path: src/main.rs id: src/main.rs#1:9c0d2e old_start: 12 new_start: 12
```

Stage the second hunk and rows 3 to 5 of the first hunk.

```nurt example
git.stage "/home/user/repo" action: add hunks: ["src/main.rs#2", "src/main.rs#1@3-5"]
```

Stage a directory without log files.

```nurt example
git.stage "/home/user/repo" action: add paths: ["src"] exclude: ["*.log"]
```

Unstage one hunk of a staged file.

```nurt example
git.stage "/home/user/repo" action: unstage hunks: ["README.md#1"]
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `add` or `unstage` without `paths` and `hunks` (`nothing_selected`). | `git.stage "/home/user/repo" action: list` |
| `E_ARG` | A path is outside the working tree or inside `.git` (`invalid_path`), or an identifier or row selection is malformed (`invalid_id`, `invalid_rows`). | Use a path relative to the working tree root |
| `E_NOT_FOUND` | A file is neither in the working tree, the index nor `HEAD` (`pathspec_no_match`), or a hunk does not exist (`not_found`). | List the hunks again |
| `E_STALE` | The fingerprint of a hunk no longer matches (`changed`), or the index changed between planning and writing. | List the hunks again |
| `E_GIT_LOCKED` | `index.lock` exists. | Wait for the other process or remove the stale lock |
| `E_GIT_UNSUPPORTED` | The repository has no working tree (`bare`). | Run the command in a working tree |

## Related

`git.status`, `git.diff`, `git.commit`, `git.restore`, `undo`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
