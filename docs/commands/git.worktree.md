---
command: git.worktree
group: git
summary: Add, list or remove linked working trees.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.worktree

## Synopsis

```text
git.worktree <repo> action: <enum(add|list|remove)> [name: value]...
```

## Description

`add` creates a linked working tree in a directory that does not exist or is
empty.
The registration is written to `.git/worktrees/<name>/` with the files `HEAD`,
`ORIG_HEAD`, `commondir`, `gitdir`, `index` and `logs/HEAD`, as `git worktree
add` writes them, and the new directory receives a `.git` file that points at
the registration. `<name>` is the last component of `path`; when the name is
taken, the smallest number that makes it unique is appended (`wt`, `wt1`,
`wt2`).

The branch is `branch`. Without `branch` and `start_point` it is the last
component of `path`. Without `branch`, a `start_point` that names a local
branch checks that branch out, and any other `start_point` is checked out
detached: `HEAD` holds the commit and the record has `detached: true`. An
existing branch is checked out and must not be checked out in
another working tree. A missing branch is created at `start_point`, or at
`HEAD`, with the log message `branch: Created from <start_point>`. The files
of the commit are written to the new directory and recorded in its index.
Content filter drivers are not run.

`list` returns the main working tree first and then the linked working trees
sorted by path, with the lock reason and whether the directory is missing
(`prunable`). It gives the same result from any working tree of the
repository.

`remove` deletes a linked working tree and its registration. A working tree
with modified, staged or untracked files that are not ignored is removed only
with `force: true`. Ignored files do not block the removal. The branch is not
deleted.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `action` | `enum(add\|list\|remove)` | required | Operation on working trees. |
| `path` | `path` | none | Directory of the working tree; a relative path is taken relative to `repo`. For `add` the directory must not exist or must be empty. |
| `branch` | `text` | none | With `add`: branch checked out in the new working tree. |
| `start_point` | `text` | none | With `add`: revision a new branch starts from; not allowed for an existing branch. Without `branch`: local branch to check out, or revision to check out detached. |
| `force` | `bool` | `false` | With `remove`: remove a working tree that has uncommitted changes or untracked files. |
| `checkpoint` | `bool` | `false` | With `remove`: record the content of the working tree as a checkpoint (`git.checkpoint`) in the same operation; the effect names it in `checkpoint`. |
| `expect_state` | `text` | none | State token from an earlier effect. The operation fails with `E_STALE` and problem `state_changed` when `HEAD`, the branches, a registration or a working tree, including the names of its ignored files, changed since. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `git_worktree`, `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |
| Reversibility conditions | Remove moves the working tree directory into the journal before unregistering it. |

| Action | Effect | Reversibility | Note |
|---|---|---|---|
| `list` | `read` | `read` |  |

## Examples

Add a working tree with a new branch.

```nurt example
git.worktree "/home/user/repo" action: add path: "/home/user/repo-hotfix" branch: "hotfix/1.2" start_point: "v1.2.0"
```

```text expected
git_worktree path: /home/user/repo-hotfix branch: hotfix/1.2 main: false
```

List the working trees of a repository.

```nurt example
git.worktree "/home/user/repo" action: list
```

```text expected
git_worktree path: /home/user/repo branch: main main: true
git_worktree path: /home/user/repo-hotfix branch: hotfix/1.2 main: false prunable: false
```

Remove a working tree that still has local changes.

```nurt example
git.worktree "/home/user/repo" action: remove path: "/home/user/repo-hotfix" force: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `action` or, with `add` and `remove`, `path` is missing. | Pass `path` |
| `E_GIT_WORKTREE` | The directory exists and is not empty (`path_exists`), a registration names it (`already_registered`), the path is the main working tree (`main_worktree`), no working tree is registered there (`not_found`), or the working tree is locked (`locked`). | `git.worktree "/home/user/repo" action: list`; remove the `locked` file of the registration |
| `E_GIT_REF` | The branch is checked out in another working tree (`checked_out_elsewhere`), `start_point` is given for an existing branch (`branch_exists`), or the branch name is invalid (`invalid_name`). | Pass another `branch` |
| `E_GIT_REVISION` | `start_point`, or `HEAD` of a repository without commits, names no commit. | Pass an existing revision as `start_point` |
| `E_GIT_DIRTY` | The working tree to remove has modified, staged or untracked files (`local_changes`). | `git.worktree "/home/user/repo" action: remove path: "/home/user/repo-hotfix" force: true` |
| `E_GIT_UNSUPPORTED` | The commit contains an unsafe path (`unsafe_path`). | Check out another revision |
| `E_GIT_LOCKED` | The lock file of the new branch exists. | Wait for the other process or remove the stale lock file |
| `E_STALE` | The branch, the directory or the registration appeared between planning and writing. | Run the command again |

## Related

`git.switch`, `git.branch`, `git.checkpoint`, `undo`.
