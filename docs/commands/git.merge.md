---
command: git.merge
group: git
summary: Merge a branch into the current branch, or continue or abort a merge.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.merge

## Synopsis

```text
git.merge <repo> branch: <text> [fast_forward: auto|only|never] [message: <text>] [no_commit: <bool>]
git.merge <repo> action: continue [message: <text>]
git.merge <repo> action: abort
```

## Description

With `action: start` (the default) the commit named by `branch` is merged
into the current branch. When the current commit is an ancestor of the
merged commit, the branch is fast-forwarded: the working tree, the index and
the branch move to the merged commit and no commit is created. When the
merged commit is already contained in the current branch, nothing changes
and the state is `up_to_date`. Otherwise a three-way merge against the best
common ancestor is computed; several best ancestors are first merged into a
virtual ancestor. Text is merged line by line with the histogram algorithm,
renames are followed according to `merge.renames` and `merge.renameLimit`,
and the `merge` and `conflict-marker-size` attributes are honoured.

A clean three-way merge creates a merge commit whose parents are the
current commit and the merged commit. Its message is `Merge branch '<name>'`,
`Merge remote-tracking branch '<name>'`, `Merge tag '<name>'` or
`Merge commit '<revision>'`, followed by ` into <branch>` unless the current
branch matches `merge.suppressDest` (by default `main` and `master`).

When the merge has conflicts, or when `no_commit` is true, the merge stops:
the working tree holds the merged files with conflict markers, the index
holds stages 1, 2 and 3 of each conflicting path, and the state files
`MERGE_HEAD`, `MERGE_MSG`, `MERGE_MODE`, `AUTO_MERGE` and `ORIG_HEAD` are
written in the format of Git, so Git can conclude the merge as well. The
call then fails with `E_GIT_CONFLICT` and `effect_applied: true`; the
`effect.git` record lists the conflicts.

`action: continue` concludes a stopped merge with a commit of the index. It
requires every conflict to be resolved and staged. The message is the
prepared `MERGE_MSG` without comment lines, or `message` when given.

`action: abort` returns the index and the working tree to `HEAD` for every
path the merge changed and removes the merge state files. Local changes of
paths that the merge did not touch are kept.

Every check that can refuse a merge runs before the first change: an
operation in progress, unresolved paths, staged changes (for a three-way
merge), local changes and untracked files in the way of the result.
Configured merge drivers (`merge.<name>.driver`) run as user programs
through the program launching service when the merge is applied; their
effects are marked irreversible.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `branch` | `text` | none | Branch, tag or commit to merge; required when `action` is `start`. |
| `action` | `enum(start\|continue\|abort)` | `start` | Start a merge, continue it after resolving conflicts, or abort it. |
| `fast_forward` | `enum(auto\|only\|never)` | `auto` | Fast-forward when possible, only fast-forward, or always create a merge commit. |
| `message` | `text` | generated | Message of the merge commit. |
| `no_commit` | `bool` | `false` | Stop before creating the merge commit, as after a conflict. |

## Input and output

The command reads no input. It produces one `effect.git` record and, when
the merge stops with conflicts, one `git_conflict` record per conflicting
path.

| Field of `effect.git` | Value |
|---|---|
| `state` | `fast_forward`, `up_to_date`, `completed`, `conflicts` or `stopped` |
| `head_before`, `head_after` | Commit of `HEAD` before and after |
| `branch` | Current branch, without `refs/heads/` |
| `conflicts` | Conflicts left in the working tree |
| `in_progress` | `merge` when the merge stopped, otherwise `none` |
| `merge_head` | Merged commit while the merge is stopped |

## Effect

Category `change`, effect `local`, reversibility `reversible`. The
working tree files, the index, the branch, `HEAD`, the reference logs and
the state files are changed in one journal operation; `undo` restores each
of them byte for byte. Objects added to the object database stay, as they
are immutable. The reference logs receive the messages of Git:
`merge <branch>: Fast-forward`, `merge <branch>: Merge made by the 'ort'
strategy.`, `commit (merge): <subject>` and `reset: moving to HEAD`.

## Examples

Merge the branch `topic` into the current branch.

```nurt example
git.merge "/home/user/project" branch: "topic"
```

```text expected
effect.git state: completed branch: main
```

Create a merge commit even when the branch could be fast-forwarded, with an
own message.

```nurt example
git.merge "/home/user/project" branch: "release/1.4" fast_forward: never message: "Merge release 1.4"
```

Conclude a merge after the conflicts were resolved and staged.

```nurt example
git.stage "/home/user/project" action: add paths: ["src/main.rs"]
git.merge "/home/user/project" action: continue
```

Abort a merge that stopped with conflicts.

```nurt example
git.merge "/home/user/project" action: abort
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_CONFLICT` | The merge stopped with conflicts (`effect_applied: true`), or the index has unresolved paths (`unmerged_paths`). | Resolve the files, stage them and run `action: continue`, or run `action: abort`. |
| `E_GIT_DIRTY` | Staged changes (`staged_changes`), local changes (`local_changes`) or untracked files are in the way. | Commit or stash the changes: `git.stash "<repo>" action: push`. |
| `E_GIT_IN_PROGRESS` | Another operation is in progress, or no merge is stopped for `continue` and `abort` (`no_merge_in_progress`). | Conclude or abort the operation in progress. |
| `E_GIT_NON_FAST_FORWARD` | `fast_forward: only` and the histories diverged (`not_possible`), or the histories are unrelated (`unrelated_histories`). | Merge with `fast_forward: auto`, or rebase onto the branch. |
| `E_GIT_REVISION` | `branch` names no commit. | Give an existing branch, tag or commit. |
| `E_GIT_LOCKED` | `index.lock` or a reference lock exists. | Wait for the other Git process or remove the lock file. |
| `E_MISSING_PARAM` | `action: start` without `branch`. | Give `branch: "<name>"`. |

## Related

`git.status`, `git.stage`, `git.commit`, `git.rebase`, `git.pull`,
`git.stash`, `undo`.
