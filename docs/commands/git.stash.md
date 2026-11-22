---
command: git.stash
group: git
summary: Save uncommitted changes in the stash, or list, show, apply, pop, drop or clear stash entries.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.stash

## Synopsis

```text
git.stash <repo: path> action: push|pop|apply|list|show|drop|clear [stash: <text>] [message: <text>]
          [include_untracked: <bool>] [paths: <list<path>>] [exclude: <list<glob>>]
          [expect_state: <text>]
```

## Description

`push` saves the staged and unstaged changes of tracked files as a new stash
entry and resets them to `HEAD`. The entry has the structure `git` uses: a
commit of the index (parent `HEAD`), with `include_untracked` a
commit of the untracked files (no parent), and a commit of the working tree
with the parents `HEAD`, the index commit and the untracked commit. Its
message is `WIP on <branch>: <commit> <subject>`, or `On <branch>: <message>`
with `message`. `refs/stash` points to the new entry and its reflog holds
the list. With `paths` only changes below the given files or directories are
saved and reset; `exclude` leaves out paths matching a pattern (a pattern
without `/` matches the file name). Nothing is saved when there are no
changes.

`apply` merges an entry into the working tree with the parent of its working
tree commit as the base; `pop` does the same and removes the entry when the
merge is clean. Changes are applied unstaged, except that files new in the
entry stay added. Untracked files of the entry are written back. When
the merge leaves conflicts, the files contain conflict markers labelled
`Updated upstream` and `Stashed changes`, the index holds the conflict
stages, `pop` keeps the entry and the command fails with `E_GIT_CONFLICT`
after the change is applied.

`drop` removes one entry and `clear` removes all of them by rewriting the
reflog of `refs/stash`; the stashed objects stay in the repository. `list`
and `show` only read.

Entries are named `stash@{N}` or `N`, 0 being the newest; `apply` and `show`
also accept a commit. Configured filter drivers are not run.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or a path inside it. |
| `action` | `enum(push\|pop\|apply\|list\|show\|drop\|clear)` | required | Operation on the stash. |
| `stash` | `text` | `stash@{0}` | Entry for `pop`, `apply`, `show` and `drop`. |
| `message` | `text` | none | With `push`: message of the new entry. |
| `include_untracked` | `bool` | `false` | With `push`: also save and remove untracked files. |
| `paths` | `list<path>` | all paths | With `push`: only changes in these files or directories. |
| `exclude` | `list<glob>` | `[]` | With `push`: paths to leave out; the report counts the excluded files. |
| `expect_state` | `text` | none | State token from an earlier read; nothing changes when `HEAD`, the index or the newest entry differ. |

## Input and output

No input. `list` produces one `git_stash` record per entry, newest first;
`show` produces the `git_stash` record of the entry. The changing actions
produce an `effect` record with the written and removed working tree paths
as changes and `git.state` `completed`, `conflicts` or `nothing_to_do`.
Notable effects: `conflicts`, `stash_kept` (a `pop` with conflicts) and
`excluded` (with the number of files left out by `exclude`).

## Effect

Category `change`, local effect, reversible; `list` and `show` read. For
`push`, `apply` and `pop` the exact previous content of every changed file,
the index, `refs/stash`, its reflog and the reflog of `HEAD` are targets of
one journal entry. For `drop` and `clear` the targets are `refs/stash`, its
reflog and, when the reference is packed, `packed-refs`. New objects are
added to the object store, which the journal does not track.

## Examples

Save all local changes, including new untracked files.

```nurt example
git.stash "/home/user/repo" action: push message: "parser rewrite" include_untracked: true
```

```text expected
effect git.stash branch: main
```

List the entries.

```nurt example
git.stash "/home/user/repo" action: list
```

```text expected
git_stash index: 0 message: "On main: parser rewrite" includes_untracked: true branch: main
```

Bring back the newest entry and remove it.

```nurt example
git.stash "/home/user/repo" action: pop
```

Save only the changes of one directory, leaving out environment files.

```nurt example
git.stash "/home/user/repo" action: push paths: ["src"] exclude: ["*.env"]
```

Remove the second newest entry.

```nurt example
git.stash "/home/user/repo" action: drop stash: "stash@{1}"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `repo` is not inside a repository. | Give the path of a working tree |
| `E_GIT_REF` | The branch has no commit yet (`unborn_branch`), the stash is empty (`no_stash`) or `pop`/`drop` names a commit outside the list (`not_a_stash_reference`). | `git.stash "/home/user/repo" action: list` |
| `E_GIT_REVISION` | The entry does not exist (`out_of_range`) or is not a stash commit (`not_a_stash`). | `git.stash "/home/user/repo" action: list` |
| `E_NO_MATCH` | A path of `paths` names no file known to git. | Check the path with `git.status "/home/user/repo"` |
| `E_GIT_IN_PROGRESS` | The index has unresolved conflicts (`unmerged_paths`). | Resolve and stage the conflicting files |
| `E_GIT_DIRTY` | Local changes would be overwritten (`local_changes`) or untracked files of the entry exist (`untracked_overwritten`); the paths are in `details.paths`. | `git.stash "/home/user/repo" action: push`, or move the files away |
| `E_GIT_CONFLICT` | The merge left conflicts (`conflicts`, effect applied). | Resolve the files listed in `details.conflicts` |
| `E_STALE` | `expect_state` differs from the current state (`state_changed`), or a file changed between planning and writing. | Read the state again |
| `E_GIT_LOCKED` | `index.lock` or a reference lock file exists. | Wait for the other process or remove the stale lock |

## Related

`git.reset`, `git.restore`, `git.status`, `git.checkpoint`, `undo`.
