---
command: git.status
group: git
summary: Show the state of the working tree and index of one or more repositories.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/git.status.md -->
<!-- Responsibility:   Documents git.status. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# git.status

## Synopsis

```text
git.status <repo>... [name: value]...
```

## Description

Lists the paths of each repository whose index differs from `HEAD`, whose
working tree file differs from the index, that have conflict stages, and that
are untracked or ignored. The entries and their states correspond to
`git status --porcelain=v2`.

The order within one repository is fixed: tracked paths and paths with
conflicts in byte order of the path, then untracked paths, then ignored
paths. Repositories follow the order of `repo`.
A path of the repository that is not valid Unicode is shown with U+FFFD in
`path` and `renamed_from` and carries its bytes in `raw_path` and
`raw_renamed_from`.

The index is compared with `HEAD` with rename and copy detection as
configured by `status.renames`, which defaults to the value of `diff.renames`
(on); a rename needs a similarity of at least 50 percent. The working tree is
compared with the index without rename detection. Paths with conflict stages
carry the kind of conflict derived from the stages present in the index.

With `untracked: normal`, a directory that contains no tracked file is one
entry whose path ends in `/`; `all` lists every untracked file and `no` none.
`ignored` adds ignored paths, collapsed in the same way. Empty directories are
not listed.

A submodule is reported as modified when its checked-out commit differs from
the recorded one, when it has modified tracked files, or when it has
untracked files; the `submodule` field tells which. Index entries outside a
sparse checkout are not compared with the working tree.

Filter drivers (`filter.<name>.clean`) are not run; files are compared after
the built-in conversions (`core.autocrlf`, `eol`, `text`, `ident`,
`working-tree-encoding`), and the report notes `filter_not_applied`.
`core.fsmonitor` is not used and is noted as `fsmonitor_ignored`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `list<path>` | required | Repositories or paths inside them. |
| `untracked` | `enum(normal\|all\|no)` | `normal` | Untracked files: directories collapsed, every file, or none. |
| `ignored` | `bool` | `false` | Include ignored files. |
| `limit` | `int` | `200` | Maximum number of entries per repository. |
| `stat` | `bool` | `false` | Include the numbers of added and removed lines. |
| `exclude` | `list<glob>` | `[]` | Paths to leave out; the report counts the excluded files. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `git_status` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Show the changes of one repository.

```nurt example
git.status "/home/user/repo"
```

```text expected
git_status path: src/main.rs index: unmodified worktree: modified kind: tracked
git_status path: notes/ index: untracked worktree: untracked kind: untracked
```

List every untracked file together with ignored files.

```nurt example
git.status "/home/user/repo" untracked: all ignored: true
```

Count changed lines in two repositories, leaving out lock files.

```nurt example
git.status ["/home/user/app", "/home/user/lib"] stat: true exclude: ["*.lock"]
```

List only the paths with conflicts after a merge.

```nurt example
git.status "/home/user/repo" untracked: no |> filter kind == "conflict"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | The path is not inside a repository, or the repository is bare (problem `bare_repository`). | Give the path of a working tree |
| `E_INVALID_VALUE` | `untracked` is not `normal`, `all` or `no`, or `limit` is outside 1 to 5000. | Use a listed value |
| `E_GIT_UNSUPPORTED` | The repository uses a format that is not implemented, such as `reftable`. | Use a repository with the files reference backend |
| `E_STAGE_FAILED` | The index, an object or a file cannot be read (problem `read_failed`). | Restore the missing objects of the repository |

## Related

`git.diff`, `git.stage`, `git.commit`, `git.review`, `git.checkpoint`, `undo`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
