---
command: git.switch
group: git
summary: Switch the working tree to a branch.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.switch

## Synopsis

```text
git.switch <repo> branch: <text> [name: value]...
```

## Description

Updates the working tree and the index to the tree of the target and points
`HEAD` at it. The update follows the two-tree rules of `git switch`:

- a path that is equal in the current commit and in the target keeps its
  index entry and its working tree file, so local changes are carried over;
- a path that differs is written from the target when its index entry and its
  working tree file agree with the current commit, and removed when the target
  lacks it; directories left empty are removed;
- a path that differs and has local changes (in the index or in the working
  tree) refuses the switch, also when the local content equals the target;
- an untracked file where the target has a file refuses the switch unless the
  file is ignored; a directory with untracked files where the target has a file
  refuses the switch.

All refusals are found before anything changes and are reported together.
The log of `HEAD` receives `checkout: moving from <old> to <new>`, also when
the target is the current branch. With `detach` the old and new names are the
branch name or the full commit id, as `git` writes them.

The target `-` selects the branch checked out before the current one, read
from the log of `HEAD`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `branch` | `text` | required | Branch to switch to. |
| `create` | `bool` | `false` | Create the branch from HEAD first. |
| `checkpoint` | `bool` | `false` | Create a checkpoint of the working tree before switching. |
| `guess` | `bool` | `false` | Create a local branch from a remote branch of the same name when no local branch exists. |
| `start_point` | `text` | none | Revision the new branch starts from; applies with create. |
| `detach` | `bool` | `false` | Detach HEAD at the revision instead of switching to a branch. |
| `discard_changes` | `bool` | `false` | Discard local changes of the working tree while switching. |

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

Switch to an existing branch.

```nurt example
git.switch "/home/user/repo" branch: "feature/login"
```

```text expected
effect git.switch head_after: 4f2a9c1 branch: feature/login
```

Create a branch at a tag and switch to it.

```nurt example
git.switch "/home/user/repo" branch: "fix/1.2" create: true start_point: "v1.2.0"
```

Check out a commit without a branch.

```nurt example
git.switch "/home/user/repo" branch: "HEAD~3" detach: true
```

Return to the previous branch.

```nurt example
git.switch "/home/user/repo" branch: "-"
```

Keep the current working tree in a checkpoint, then switch.

```nurt example
git.switch "/home/user/repo" branch: "main" checkpoint: true
```

```text expected
effect git.switch head_after: 9c03e7a branch: main checkpoint_before: cp-4
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_DIRTY` | Local changes, untracked files or unresolved conflicts block the switch; the details list the paths by kind (`local_changes`, `untracked_overwritten`, `untracked_removed`, `dir_loses_untracked`, `unmerged`). | `git.stash "/home/user/repo" action: push`, or `discard_changes: true` |
| `E_GIT_REF` | The branch exists (`create`), the name is invalid, or the name is a commit without `detach` (`expected_branch`). | Drop `create`, or add `detach: true` |
| `E_ARG` | `start_point` without `create` (`start_point_without_create`). | Add `create: true` |
| `E_GIT_REVISION` | The branch or start point does not exist. | `git.branch "/home/user/repo"` to list branches |
| `E_GIT_IN_PROGRESS` | A merge, rebase, cherry-pick or revert is in progress. | Finish or abort the operation |
| `E_GIT_LOCKED` | `index.lock` or a reference lock exists. | Wait for the other process or remove the stale lock |
| `E_STALE` | A file changed between planning and writing. | Run the command again |

## Related

`git.restore`, `git.branch`, `git.stash`, `git.checkpoint`, `undo`.
