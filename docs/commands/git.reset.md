---
command: git.reset
group: git
summary: Move the current branch to a revision, optionally resetting the index and working tree.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.reset

## Synopsis

```text
git.reset <repo> revision: <text> [name: value]...
```

## Description

Moves the current branch, or a detached `HEAD`, to the commit named by
`revision`. The mode decides what else is reset:

- `soft` changes only the reference. It is refused while a merge is in
  progress or the index has conflicts.
- `mixed` also replaces the index with the tree of the revision. Index
  entries whose object and mode do not change keep their file status data;
  the working tree is not touched.
- `hard` also brings the working tree to the revision. Local changes of
  tracked files are discarded and an untracked file in the way of a tracked
  file of the revision is overwritten. Other untracked files stay.

The previous commit of `HEAD` is written to `ORIG_HEAD`. The reflog of the
branch receives `reset: moving to <revision>` when the branch moves; the
reflog of `HEAD` receives the same entry. The state files of a merge,
cherry-pick or revert in progress (`MERGE_HEAD`, `MERGE_MSG`, `MERGE_MODE`,
`MERGE_RR`, `AUTO_MERGE`, `SQUASH_MSG`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`)
are removed. Configured filter drivers are not run; the built-in line
ending, `ident` and working tree encoding conversions are applied.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or a path inside it. |
| `revision` | `text` | required | Commit to move to: a branch, tag, commit or expression such as `HEAD~2`. |
| `mode` | `enum(soft\|mixed\|hard)` | `mixed` | What is reset besides the reference. |

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
| Reversibility conditions | The previous reference and the exact previous content of every changed file are kept in the journal. |

## Examples

Undo the last commit and keep its changes staged.

```nurt example
git.reset "/home/user/repo" revision: "HEAD~1" mode: soft
```

```text expected
effect git.reset branch: main notable: history_rewritten
```

Unstage everything, keeping the working tree.

```nurt example
git.reset "/home/user/repo" revision: "HEAD"
```

Discard all local changes of tracked files and return to the remote branch.

```nurt example
git.reset "/home/user/repo" revision: "origin/main" mode: hard
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `repo` is not inside a repository. | Give the path of a working tree |
| `E_GIT_UNSUPPORTED` | The repository is bare (`bare_repository`) or the index is sparse (`sparse_index`). | Use a working tree with a full index |
| `E_GIT_REVISION` | The revision does not exist (`unknown`) or is not a commit (`not_a_commit`). | Check the name with `git.log "/home/user/repo"` |
| `E_GIT_IN_PROGRESS` | `soft` during a merge (`merge_in_progress`). | `git.reset "/home/user/repo" revision: "HEAD" mode: mixed` |
| `E_GIT_LOCKED` | `index.lock` or a reference lock file exists. | Wait for the other process or remove the stale lock |
| `E_UNKNOWN_VALUE` | `mode` is not one of the modes. | Use `soft`, `mixed` or `hard` |
| `E_STALE` | A file changed between planning and writing. | Run the command again |

## Related

`git.restore`, `git.switch`, `git.stash`, `git.log`, `undo`.
