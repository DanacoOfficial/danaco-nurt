---
command: git.checkpoint
group: git
summary: Create, list, compare, restore or delete checkpoints of the working tree and index kept outside branches.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.checkpoint

## Synopsis

```text
git.checkpoint <repo> action: create [label: <text>] [index: <bool>] [exclude: <list<glob>>]
git.checkpoint <repo> action: list
git.checkpoint <repo> action: compare checkpoint_id: <text> [against: <text>] [stat: <bool>] [context: <int>] [paths: <list<path>>]
git.checkpoint <repo> action: restore checkpoint_id: <text> [paths: <list<path>>] [index: <bool>] [save_current: <bool>] [expect_state: <text>]
git.checkpoint <repo> action: delete (checkpoint_id: <text | list<text>> | keep: <int>)
```

## Description

A checkpoint is a commit that records the working tree as `git add -A` would
stage it in a temporary index: tracked files with their working tree content
and untracked files that are not ignored. Ignored files are not recorded. With
`index: true` a second commit records the index and becomes the last parent of
the checkpoint commit, as in a stash entry. The first parent is `HEAD`; a
repository without commits gives a checkpoint without parents.

Checkpoints are stored as `refs/nurt/checkpoints/cp-<N>`, numbered from 1 in
increasing order. Creating a checkpoint does not change `HEAD`, the index,
branches or the working tree. The commit message has the form

```text
nurt checkpoint cp-3: before refactoring

branch: main
head: 5b6f0e3c0a2d7c1b0d1e9f6a4c3b2a1908f7e6d5
index: none
```

Checkpoints under `refs/danaco/checkpoints/` are listed, compared and
restored, but not deleted; new numbers continue after the highest number of
both prefixes.

`list` returns the checkpoints newest first. `compare` computes the difference
between the checkpoint and the `against` side: another checkpoint (`cp-<N>`),
any revision, or the working tree when `against` is absent. Hunks use the
Myers algorithm with the indent heuristic of `git diff`. `restore` writes the
files of the checkpoint that differ from the working tree, removes tracked and
untracked files that are not ignored and are absent from the checkpoint, and
removes directories left empty by the removals. `HEAD` and branches do not
change. With `index: true` the index entries within `paths` are replaced by
the recorded index. `delete` removes the named checkpoints, or all but the
`keep` newest ones.

Content filter drivers configured by the repository are not run; line ending
conversion, `ident` and `working-tree-encoding` are applied.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `action` | `enum(create\|list\|compare\|restore\|delete)` | required | Operation on checkpoints. |
| `label` | `text` | `checkpoint` | Label of a new checkpoint; only the first line is kept. |
| `checkpoint_id` | `text \| list<text>` | none | Checkpoint to compare or restore, as `cp-<N>`, or checkpoints to delete. |
| `against` | `text` | the working tree | With `compare`: checkpoint or revision on the new side. |
| `stat` | `bool` | `false` | With `compare`: one change per file with added and removed lines instead of hunks. |
| `context` | `int` | `3` | With `compare`: lines of context around each change, from 0 to 200. |
| `function_context` | `bool` | `false` | With `compare`: not available for checkpoints; `true` fails with `E_GIT_UNSUPPORTED`. |
| `symbol` | `text` | none | With `compare`: not available for checkpoints; fails with `E_GIT_UNSUPPORTED`. |
| `word_diff` | `bool` | `false` | With `compare`: not available for checkpoints; `true` fails with `E_GIT_UNSUPPORTED`. |
| `ignore_whitespace` | `bool` | `false` | With `compare`: not available for checkpoints; `true` fails with `E_GIT_UNSUPPORTED`. |
| `ignore_moved` | `bool` | `false` | With `compare`: not available for checkpoints; `true` fails with `E_GIT_UNSUPPORTED`. |
| `paths` | `list<path>` | all files | With `compare` and `restore`: files or directories, relative to the root of the working tree or absolute inside it. |
| `index` | `bool` | `false` | With `create`: also record the index. With `restore`: also restore the recorded index. |
| `keep` | `int` | none | With `delete`: keep only this many newest checkpoints; excludes `checkpoint_id`. |
| `save_current` | `bool` | `true` | With `restore`: record the current state as a new checkpoint labelled `before restore of cp-<N>` in the same operation, unless the restore changes nothing. The effect names it in `checkpoint_before`. |
| `exclude` | `list<glob>` | `[]` | With `create`: patterns of files left out of the checkpoint, tracked or not. The patterns are stored in the commit message as `exclude:` lines; `restore` leaves matching files untouched. The report counts the excluded files (`excluded`). |

## Input and output

The command takes no input. `create` produces one `git_checkpoint` record,
`list` produces one `git_checkpoint` record per checkpoint, `compare` produces
`hunk` records, or `change` records with `stat: true`, and `restore` and
`delete` produce one `effect` record.

## Effect

Category `change`, local effect, reversible; `list` and `compare` only read.
`create` writes new objects, which are exempt from the journal, and one
reference by the lock protocol of `git`; `undo` removes the reference.
`restore` changes every written or removed file and the index in one journal
entry; `undo` restores their previous content exactly. The report lists the
number of removed untracked files as `untracked_removed`. `delete` removes the
loose reference, its line in `packed-refs` and its log; `undo` restores all
three.

Symbolic links recorded in a checkpoint are not written by `restore`, and the
executable bit of a restored file is not changed.

## Examples

Record the working tree and the index before a change.

```nurt example
git.checkpoint "/home/user/repo" action: create label: "before refactoring" index: true
```

```text expected
git_checkpoint id: cp-3 ref: refs/nurt/checkpoints/cp-3 label: before refactoring index_included: true
```

Show which files changed since a checkpoint.

```nurt example
git.checkpoint "/home/user/repo" action: compare checkpoint_id: "cp-3" stat: true
```

```text expected
change path: src/main.rs action: modify added: 4 removed: 1
```

Restore one directory from a checkpoint.

```nurt example
git.checkpoint "/home/user/repo" action: restore checkpoint_id: "cp-3" paths: ["src"]
```

Keep the ten newest checkpoints and delete the others.

```nurt example
git.checkpoint "/home/user/repo" action: delete keep: 10
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `action`, `checkpoint_id` or, with `delete`, both `checkpoint_id` and `keep` are missing. | `git.checkpoint "/home/user/repo" action: delete keep: 10` |
| `E_ARG` | `checkpoint_id` and `keep` are both given (`param_conflict`), an identifier is malformed (`invalid_id`), or a path names the `.git` directory. | Pass one selection; use the form `cp-1` |
| `E_ARITY` | More than one checkpoint is given to `compare` or `restore`. | Pass a single identifier |
| `E_GIT_REF` | The checkpoint does not exist (`not_found`) or was written by another tool (`read_only`). | `git.checkpoint "/home/user/repo" action: list` |
| `E_GIT_CHECKPOINT` | `index: true` is requested from a checkpoint without a recorded index (`no_index`). | Restore without `index` |
| `E_GIT_REVISION` | `against` names neither a checkpoint nor a revision. | Pass an existing revision |
| `E_GIT_UNSUPPORTED` | A comparison option is not available (`diff_option`), the checkpoint holds a symbolic link to restore (`symlink_restore`), or the repository is bare. | `git.diff` with the checkpoint reference; `git.restore` |
| `E_GIT_NOT_REPOSITORY` | `repo` is not inside a repository, or a path lies outside the working tree (`path_outside_repo`). | Pass a path inside the working tree |
| `E_GIT_LOCKED` | A reference, `packed-refs` or the index is locked by another process. | Wait for the other process or remove the stale lock file |
| `E_STALE` | A file, a reference or the index changed between planning and writing. | Run the command again |

## Related

`git.stash`, `git.diff`, `git.restore`, `undo`.
