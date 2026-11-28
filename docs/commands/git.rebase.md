---
command: git.rebase
group: git
summary: Replay the commits of the current branch onto another revision, or continue, skip or abort a rebase.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/git.rebase.md -->
<!-- Responsibility:   Documents git.rebase. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# git.rebase

## Synopsis

```text
git.rebase <repo> [name: value]...
```

## Description

With `action: start` (the default) the commits of `merge-base(onto, HEAD)..HEAD`
are replayed onto the commit named by `onto`, as `git rebase <onto>` does with
the `merge` back end. Merge commits are left out, and so are commits whose
patch identifier equals that of a commit already in `onto`. The commits are
replayed oldest first: each one is merged onto the result of the previous
one with the three-way merge of `git.merge`, and the new commit keeps the
author, the author date and the message of the original commit; the
committer and the commit date come from the environment of the session. A
commit whose change is already present in the new base is dropped; a commit
that was empty from the start is kept. When `HEAD` already contains `onto` on
a line of history without merges, nothing changes and the state is
`up_to_date`; when the current branch has no commits of its own, it is moved
to `onto` and the state is `fast_forward`.

The replay runs in the object database. The working tree and the index are
written once, from their state before the call to the final state, and the
branch is moved to the last replayed commit.

When a commit cannot be replayed without conflicts, the rebase stops:
`HEAD` is detached at the last replayed commit, the working tree holds the
merged files with conflict markers labelled `HEAD` and `<short> (<subject>)`,
and the index holds stages 1, 2 and 3 of each conflicting path. The state
directory `rebase-merge/` and the files `REBASE_HEAD`, `MERGE_MSG`,
`AUTO_MERGE` and `ORIG_HEAD` are written in the format of Git 2.53, so that
Git can continue the rebase as well, and a rebase stopped by Git can be
continued here. The call fails with `E_GIT_CONFLICT` and
`effect_applied: true`; the details hold the conflicts and the step
(`done`, `total`). A commit whose result would overwrite an untracked file
stops the rebase in the same way with `E_GIT_DIRTY`
(`untracked_overwritten`); the commit stays first in the todo list.

`action: continue` commits the resolved index with the author and the
message of the stopped commit, without the `# Conflicts:` section, and
replays the remaining commits. Every conflict must be resolved and staged,
and the working tree must have no unstaged changes. When the index equals
`HEAD`, no commit is created.

`action: skip` drops the stopped commit, discards its changes from the
index and the working tree, and replays the remaining commits.

`action: abort` returns the working tree, the index and `HEAD` to the state
before the rebase and removes the state files.

Every check that can refuse a rebase runs before the first change: an
operation in progress, unresolved paths, staged and unstaged changes, and
untracked files in the way of `onto`. Hooks are not run (`hooks_skipped`),
and merge driver programs are not started during a rebase.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `onto` | `text` | none | Revision to rebase onto; required when action is start. |
| `action` | `enum(start\|continue\|skip\|abort)` | `start` | Start a rebase, continue it, skip the current commit, or abort it. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect`, `git_conflict` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |
| Reversibility conditions | The previous references and the exact previous content of every changed file are kept in the journal. |

## Examples

Replay the commits of the current branch onto `main`.

```nurt example
git.rebase "/home/user/project" onto: "main"
```

```text expected
effect.git state: completed branch: topic
```

Rebase onto the remote-tracking branch after a fetch.

```nurt example
git.fetch "/home/user/project"
git.rebase "/home/user/project" onto: "origin/main"
```

Continue a rebase after the conflicts were resolved and staged.

```nurt example
git.stage "/home/user/project" action: add paths: ["/home/user/project/src/main.rs"]
git.rebase "/home/user/project" action: continue
```

Drop the commit at which the rebase stopped.

```nurt example
git.rebase "/home/user/project" action: skip
```

Abort a rebase and return to the branch as it was.

```nurt example
git.rebase "/home/user/project" action: abort
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_CONFLICT` | The rebase stopped with conflicts (`effect_applied: true`), or the index has unresolved paths (`unmerged_paths`). | Resolve the files, stage them and run `action: continue`, or run `action: skip` or `action: abort`. |
| `E_GIT_DIRTY` | Staged changes (`staged_changes`), unstaged changes (`unstaged_changes`) or untracked files in the way (`untracked_overwritten`); with `effect_applied: true` the rebase stopped at a commit. | Commit or stash the changes: `git.stash "<repo>" action: push`; move the untracked files away. |
| `E_GIT_IN_PROGRESS` | Another operation is in progress, or no rebase is stopped for `continue`, `skip` and `abort` (`no_rebase_in_progress`). | Conclude or abort the operation in progress. |
| `E_GIT_REVISION` | `onto` names no commit. | Give an existing branch, tag or commit. |
| `E_GIT_REF` | `HEAD` has no commit (`unborn_branch`). | Create a first commit. |
| `E_GIT_SIGN` | `commit.gpgSign` is set; replayed commits cannot be signed. | Set `commit.gpgSign` to `false` for the repository. |
| `E_GIT_UNSUPPORTED` | `rebase.updateRefs` is set, `rebase.autoStash` is set with uncommitted changes, the todo list has interactive commands, or the stopped rebase uses the `apply` back end. | Conclude the rebase with Git, or change the setting. |
| `E_GIT_LOCKED` | `index.lock` or a reference lock exists. | Wait for the other Git process or remove the lock file. |
| `E_MISSING_PARAM` | `action: start` without `onto`. | Give `onto: "<revision>"`. |

## Related

`git.merge`, `git.pull`, `git.status`, `git.stage`, `git.stash`,
`git.log`, `undo`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
