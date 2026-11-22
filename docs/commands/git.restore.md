---
command: git.restore
group: git
summary: Restore files or selected hunks in the working tree or index from a revision.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.restore

## Synopsis

```text
git.restore <repo: path> paths: <list<path>> [source: <text>] [staged: <bool>] [worktree: <bool>] [hunks: <list<text>>]
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

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or a path inside it. |
| `paths` | `list<path>` | required | Files, directories or patterns to restore. |
| `source` | `text` | the index, or `HEAD` with `staged` | Revision to restore from. |
| `staged` | `bool` | `false` | Restore the index. |
| `worktree` | `bool` | `true` | Restore the working tree. |
| `hunks` | `list<text>` | none | Only these hunks, by identifier as in `git.stage`. |

## Input and output

No input. The output is an `effect` record listing the restored and removed
paths.

## Effect

Category `change`, local effect, reversible. Every changed working tree file
and the index are targets of one journal entry; `undo` restores the previous
content exactly, including local changes that the restoration overwrote.
References and their logs are not changed.

## Examples

Discard the local changes of one file.

```nurt example
git.restore "/home/user/repo" paths: ["src/main.rs"]
```

```text expected
effect git.restore written: src/main.rs
```

Unstage every change below a directory, keeping the working tree.

```nurt example
git.restore "/home/user/repo" paths: ["docs"] staged: true worktree: false
```

Take the files of a directory from an older commit into the index and the
working tree.

```nurt example
git.restore "/home/user/repo" paths: ["config"] source: "HEAD~2" staged: true worktree: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NO_MATCH` | A pathspec matched no file of the source or the index. | Check the path with `git.status "/home/user/repo"` |
| `E_GIT_REVISION` | The source does not exist or is not a tree. | Give an existing commit, tag or tree |
| `E_GIT_DIRTY` | A selected path has unresolved conflicts (`unmerged`). | Restore with `staged: true source: "HEAD"` |
| `E_ARG` | Neither `staged` nor `worktree` is set, or a pathspec is malformed. | Set `worktree: true` |
| `E_GIT_LOCKED` | `index.lock` exists. | Wait for the other process or remove the stale lock |
| `E_STALE` | A file changed between planning and writing. | Run the command again |

## Related

`git.switch`, `git.stage`, `git.reset`, `git.checkpoint`, `undo`.
