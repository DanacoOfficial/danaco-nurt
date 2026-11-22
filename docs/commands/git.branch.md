---
command: git.branch
group: git
summary: List branches, or create, rename, switch or delete them.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.branch

## Synopsis

```text
git.branch <repo: path> [remote: <bool>] [trash: <bool>]
git.branch <repo: path> create: <text> [from: <text>] [switch: <bool>] [checkpoint: <bool>] [expect_state: <text>]
git.branch <repo: path> rename: <text> new_name: <text> [check_references: <bool>] [expect_state: <text>]
git.branch <repo: path> delete: <text | list<text>> [force: <bool>] [check_references: <bool>] [expect_state: <text>]
git.branch <repo: path> purge: <list<text> | all>
```

## Description

Without `create`, `rename`, `delete` or `purge` the command reads the
repository and lists the local branches in the order of `git branch --list`,
or the remote-tracking branches with `remote: true`. Symbolic remote
references such as `origin/HEAD` are not listed. Each local branch carries its
upstream, the numbers of commits ahead of and behind it, `upstream_gone` when
the configured upstream no longer exists, and the working tree in which it is
checked out.

`create` writes `refs/heads/<name>` at the commit of `from` and the reflog
entry `branch: Created from <from>`. When `from` names a remote-tracking branch
of a configured remote and `branch.autoSetupMerge` is not `false`, the
configuration receives `branch.<name>.remote` and `branch.<name>.merge`, as
`git branch` writes them. With `switch: true` the working tree, the index and
`HEAD` move to the new branch by the rules of `git switch -c`: local changes
that do not conflict are carried over, and the reflog of `HEAD` receives
`checkout: moving from <old> to <name>`. `checkpoint: true` records the working
tree and the index as a checkpoint (`git.checkpoint`) in the same operation,
before the switch.

`rename` moves the reference, its reflog and the `branch.<name>` section of
the configuration, and moves `HEAD` of every working tree that points to the
branch, writing `Branch: renamed <old> to <new>` to the reflogs as
`git branch -m` does.

`delete` removes branches and their reflogs, also from `packed-refs`. A branch
is merged when its commit is reachable from its upstream, or from `HEAD` when
it has no upstream; a branch that is not merged is deleted only with
`force: true` and is reported as `branch_deleted_unmerged`. A branch checked
out in this or another working tree is never deleted. When the configuration
key `git.trash_refs` is `true`, the value of each deleted branch is kept as
`refs/nurt/trash/<name>-<seconds>`; `trash: true` lists the kept copies and
`purge` removes them, or all of them with `purge: all`. A `purge` name selects
a copy by its reference, by the last component of the reference
(`<name>-<seconds>`), or every copy of a deleted branch by the branch name.

With `check_references` the command reports references that still name a
renamed or deleted branch: symbolic references to it and `branch.<name>.merge`
entries of branches that track it with `remote = .`.

`expect_state` takes the `state_token` of an earlier `git.branch` effect.
The token covers `HEAD`, `packed-refs` and the local branches; when it differs
nothing changes and the command fails with `E_STALE`.

Every reference file is written under a Git-compatible `<file>.lock`
protocol through the journal; an existing lock fails with `E_GIT_LOCKED`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or a path inside it. |
| `remote` | `bool` | `false` | List remote-tracking branches. |
| `trash` | `bool` | `false` | List the kept copies of deleted branches instead of the branches. |
| `create` | `text` | none | Name of a branch to create. |
| `from` | `text` | `"HEAD"` | Start point of the new branch: a branch, tag, commit or remote-tracking branch. |
| `switch` | `bool` | `false` | Check out the created branch. Requires `create`. |
| `checkpoint` | `bool` | `false` | Create a checkpoint of the working tree before switching. Requires `switch`. |
| `rename` | `text` | none | Branch to rename. |
| `new_name` | `text` | none | New name of the renamed branch; required with `rename`. |
| `delete` | `text \| list<text>` | none | Branches to delete. |
| `force` | `bool` | `false` | Delete branches that are not merged. |
| `purge` | `list<text> \| all` | none | Kept copies to remove, by reference, copy name or branch name. |
| `check_references` | `bool` | `true` | After `delete` or `rename`, report other references that still name the old branch. |
| `expect_state` | `text` | none | State token of an earlier effect; nothing changes when the state differs. |

`create`, `rename`, `delete` and `purge` exclude each other.

## Input and output

No input. A listing produces `branch` records with the fields `name`,
`current`, `commit`, `remote`, `upstream`, `ahead`, `behind`,
`upstream_gone`, `worktree`, `subject` and `time`. `trash: true` produces
`git_trash_entry` records with the fields `name` (the deleted branch), `kind`
(`branch`), `ref`, `commit`, `time` (of the deletion) and `subject`. A change
produces one
`effect` record with `head_before`, `head_after`, `branch`, `detached`,
`checkpoint_before` and `state_token`, and the notable effects
`branch_deleted_unmerged`, `references_left` and `local_changes_carried`.

## Effect

Without a changing parameter the command only reads. `create`, `rename`,
`delete` and `purge` are local and reversible: every reference file, reflog,
`packed-refs`, the configuration and, with `switch`, every changed working
tree file and the index are targets of one journal entry, and `undo` restores
them byte for byte. Objects of a checkpoint are added to the object database.

## Examples

List the local branches with their upstreams.

```nurt example
git.branch "/home/user/repo"
```

```text expected
branch name: main current: true upstream: origin/main ahead: 1 behind: 0
```

Create a branch at a tag and check it out, keeping a checkpoint first.

```nurt example
git.branch "/home/user/repo" create: "fix/1.2" from: "v1.2.0" switch: true checkpoint: true
```

Rename the current branch.

```nurt example
git.branch "/home/user/repo" rename: "master" new_name: "main"
```

Delete a branch that was never merged.

```nurt example
git.branch "/home/user/repo" delete: "spike" force: true
```

List the kept copies of deleted branches.

```nurt example
git.branch "/home/user/repo" trash: true
```

```text expected
git_trash_entry name: spike kind: branch ref: refs/nurt/trash/spike-1767225600
```

Remove all kept copies of deleted branches.

```nurt example
git.branch "/home/user/repo" purge: all
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_REF` | The name is invalid (`invalid_name`), the branch exists (`exists`), does not exist (`missing`), is checked out (`current_branch`, `checked_out`) or is not merged (`not_merged`). | `git.branch "/home/user/repo" delete: "spike" force: true` |
| `E_GIT_REVISION` | `from` does not resolve to a commit. | `git.log "/home/user/repo"` |
| `E_GIT_DIRTY` | With `switch`, local changes would be overwritten. | `git.stash "/home/user/repo" action: push` |
| `E_GIT_LOCKED` | A reference lock exists. | Wait for the other process or remove the stale lock |
| `E_STALE` | A reference changed between planning and writing, or `expect_state` differs. | Read the branches again and repeat the call |
| `E_ARG` | Parameters exclude each other (`param_conflict`). | Pass one changing parameter per call |
| `E_MISSING_PARAM` | `rename` without `new_name`. | Add `new_name` |

## Related

`git.switch`, `git.tag`, `git.checkpoint`, `git.status`, `undo`.
