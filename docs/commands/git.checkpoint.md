---
command: git.checkpoint
group: git
summary: Create, list, compare, restore or delete checkpoints of the working tree and index kept outside branches.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/git.checkpoint.md -->
<!-- Responsibility:   Documents git.checkpoint. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# git.checkpoint

## Synopsis

```text
git.checkpoint <repo> action: <enum(create|list|compare|restore|delete)> [name: value]...
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
| `label` | `text` | none | Label of a new checkpoint. |
| `checkpoint_id` | `text \| list<text>` | none | Checkpoint to compare or restore, or checkpoints to delete. |
| `against` | `text` | the working tree | With compare: revision or checkpoint to compare with. |
| `stat` | `bool` | `false` | With compare: only changed files with added and removed lines. |
| `context` | `int` | `3` | With compare: lines of context around each change. |
| `function_context` | `bool` | `false` | With compare: extend each change to the whole enclosing function. |
| `symbol` | `text` | none | With compare: only changes within this code element. |
| `word_diff` | `bool` | `false` | With compare: show changes word by word. |
| `ignore_whitespace` | `bool` | `false` | With compare: ignore changes in whitespace. |
| `ignore_moved` | `bool` | `false` | With compare: hide blocks of lines that were only moved. |
| `paths` | `list<path>` | none | Only these files or directories. |
| `index` | `bool` | `false` | With restore: also restore the index. |
| `keep` | `int` | none | With delete: keep only this many newest checkpoints. |
| `save_current` | `bool` | `true` | Create a checkpoint of the current changes before restore. |
| `exclude` | `list<glob>` | `[]` | Paths to leave out of create; the report counts the excluded files. |
| `expect_state` | `text` | none | State token from an earlier read; nothing changes when the repository state differs. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `git_checkpoint`, `hunk`, `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |
| Reversibility conditions | Restore keeps the exact previous content of every file it changes or removes. |

| Action | Effect | Reversibility | Note |
|---|---|---|---|
| `list` | `read` | `read` |  |
| `compare` | `read` | `read` |  |

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
| `E_GIT_UNSUPPORTED` | A comparison option is not available (`diff_option`) or the repository is bare. | `git.diff` with the checkpoint reference |
| `E_GIT_NOT_REPOSITORY` | `repo` is not inside a repository, or a path lies outside the working tree (`path_outside_repo`). | Pass a path inside the working tree |
| `E_GIT_LOCKED` | A reference, `packed-refs` or the index is locked by another process. | Wait for the other process or remove the stale lock file |
| `E_STALE` | A file, a reference or the index changed between planning and writing. | Run the command again |

## Related

`git.stash`, `git.diff`, `git.restore`, `undo`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
