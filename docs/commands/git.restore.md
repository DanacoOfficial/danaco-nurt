---
command: git.restore
group: git
summary: Restore files or selected hunks in the working tree or index from a revision.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/git.restore.md -->
<!-- Responsibility:   Documents git.restore. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# git.restore

## Synopsis

```text
git.restore <repo> paths: <list<path>> [name: value]...
```

## Description

Copies the paths selected by `paths` from a source into the working tree, the
index or both. Paths use `git` pathspec syntax relative to the repository
root: a directory selects everything below it, `*.txt` and `:(glob)**/x` are
patterns. A pathspec that matches no path of the source or the index is an
error and nothing changes.

The source is the revision `source` (a commit, tag or tree). Without it the
working tree is restored from the index, and the index (with `staged`) from
`HEAD`; with `staged` and `worktree` together both come from `HEAD`.

Restoration does not overlay: a selected path present in the index but absent
from the source is removed from the restored side. The working tree is written
only where its content differs from the source. A path with unresolved
conflicts cannot be restored from the index into the working tree; restore it
with `staged: true` from a revision instead.

With `hunks` only the named hunks are taken back, as `git restore --patch`
does. Identifiers have the form of `git.stage` (`path#number`, optionally
`:fingerprint` and `@rows`). With `worktree` the hunks are those between the
index and the working tree file; with `staged: true worktree: false` they are
those between `HEAD` and the index, and a new file whose only hunk is taken
back leaves the index. Unselected hunks stay as they are. Every hunk must lie
in a file at or below one of `paths`, compared literally. `source` and the
restoration of both sides cannot be combined with `hunks`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `paths` | `list<path>` | required | Files or directories to restore. |
| `source` | `text` | the index, or HEAD with staged | Revision to restore from. |
| `staged` | `bool` | `false` | Restore the index. |
| `worktree` | `bool` | true, or false with staged | Restore the working tree. |
| `hunks` | `list<text>` | none | Only these hunks, by identifier as in git.stage. |
| `checkpoint` | `bool` | `false` | Create a checkpoint of the changes before they are discarded. |
| `expect_state` | `text` | none | State token from an earlier read; nothing changes when the repository state differs. |

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
| Reversibility conditions | The exact previous content of every changed file is kept in the journal. |

## Examples

Discard the local changes of one file.

```nurt example
git.restore "/home/user/repo" paths: ["/home/user/repo/src/main.rs"]
```

```text expected
effect git.restore written: /home/user/repo/src/main.rs
```

Unstage every change below a directory, keeping the working tree.

```nurt example
git.restore "/home/user/repo" paths: ["/home/user/repo/docs"] staged: true worktree: false
```

Take the files of a directory from an older commit into the index and the
working tree.

```nurt example
git.restore "/home/user/repo" paths: ["/home/user/repo/config"] source: "HEAD~2" staged: true worktree: true
```

Take back the second hunk of a file, keeping its other local changes.

```nurt example
git.restore "/home/user/repo" paths: ["/home/user/repo/src/main.rs"] hunks: ["src/main.rs#2"]
```

Unstage one hunk and keep a checkpoint of the state before it.

```nurt example
git.restore "/home/user/repo" paths: ["/home/user/repo/src/lib.rs"] hunks: ["src/lib.rs#1"] staged: true worktree: false checkpoint: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NO_MATCH` | A pathspec matched no file of the source or the index. | Check the path with `git.status "/home/user/repo"` |
| `E_GIT_REVISION` | The source does not exist or is not a tree. | Give an existing commit, tag or tree |
| `E_GIT_DIRTY` | A selected path has unresolved conflicts (`unmerged`). | Restore with `staged: true source: "HEAD"` |
| `E_ARG` | Neither `staged` nor `worktree` is set, a pathspec or a hunk identifier is malformed, or a hunk lies outside `paths` (`hunk_outside_paths`). | Set `worktree: true`, or add the file of the hunk to `paths` |
| `E_INVALID_VALUE` | An absolute path is outside the repository (`outside_repository`). | Give a path inside the working tree |
| `E_GIT_UNSUPPORTED` | `hunks` together with `source` (`hunks_source`), with both sides (`hunks_both`), or for a submodule (`hunks_submodule`). | Remove `source`, or set `worktree: false` for staged hunks |
| `E_NOT_FOUND` | A hunk number or the change of a hunk does not exist (`not_found`). | List the hunks again with `git.stage "/home/user/repo" action: list` |
| `E_GIT_LOCKED` | `index.lock` exists. | Wait for the other process or remove the stale lock |
| `E_STALE` | A file changed between planning and writing, the fingerprint of a hunk no longer matches (`changed`), or the state differs from `expect_state` (`state_changed`). | Read the state or the hunks again and repeat the command |

## Related

`git.switch`, `git.stage`, `git.reset`, `git.checkpoint`, `undo`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
