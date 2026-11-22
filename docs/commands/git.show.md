---
command: git.show
group: git
summary: Show commits with their changes, or the content of a file at a revision.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.show

## Synopsis

```text
git.show <repo: path> <revision: text> [revs: <list<text>>] [paths: <list<path>>] [stat: <bool>] [context: <int>] [exclude: <list<glob>>]
git.show <repo: path> <revision: text> file: <path> [offset: <int>] [limit: <int>]
```

## Description

Shows each object named by `revision` and `revs`, in that order. A commit is
shown as a `commit` record followed by one `hunk` record per change against
its first parent, with renamed files detected as `git show` detects them; a
root commit is compared with the empty tree.

A merge is shown with the condensed combined difference of `git show`
(`--cc`): only files that differ from every parent are considered, each line
carries one prefix column per parent, and a group of changes is left out when
the parents hold at most two versions of it and the merge takes one of them
unchanged. A merge that only takes changes from its parents therefore shows
no hunks. The `old_start` and `old_len` fields of a combined hunk give the
lines of the first parent.

An annotated tag is shown as a `git_tag` record followed by the object it
tags; tags of tags are followed to the end. A revision that names a tree or a
file is rejected; the content of a file is read with `file`.

With `file`, the content of the file at the commit of `revision` is returned
as one `content` record. `offset` and `limit` select lines; the content is
read as stored in the repository, without conversion filters. Binary content
is reported without text.

A change without line differences, such as a change of mode or an exact
rename, is one hunk without lines. A binary change is one hunk with
`binary: true` and no lines.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `revision` | `text` | required | Revision to show. |
| `revs` | `list<text>` | `[]` | Further revisions shown after `revision`. |
| `file` | `path` | none | Return the content of this file at the revision instead of the changes. |
| `paths` | `list<path>` | none | Only changes in these files or directories. |
| `offset` | `int` | `1` | First line of the content returned with `file`, counted from 1. |
| `limit` | `int` | none | Largest number of lines of the content returned with `file`. |
| `stat` | `bool` | `false` | Only changed files with added and removed lines, in the `commit` record; a merge is compared with its first parent. |
| `context` | `int` | `3` | Unchanged lines around each change. |
| `exclude` | `list<glob>` | `[]` | Paths to leave out; a pattern without `/` matches the file name in any directory. |

## Input and output

No input. The output is a sequence of `git_tag`, `commit` and `hunk` records,
or one `content` record with `file`.

| Record | Field | Description |
|---|---|---|
| `commit` | as in `git.log` | With `files` and `stats` when `stat` is set. |
| `hunk` | `path`, `old_path` | Path after the change, relative to the repository root, and the path before a rename. |
| `hunk` | `old_start`, `old_len`, `new_start`, `new_len` | Line ranges as written after `@@`. |
| `hunk` | `lines` | Lines with the prefixes `+`, `-` and space; one prefix column per parent for a merge. |
| `hunk` | `header` | Section heading: the nearest preceding line that starts with a letter, `_` or `$`. |
| `hunk` | `status` | `added`, `deleted`, `modified`, `renamed` or `binary`. |
| `hunk` | `old_mode`, `new_mode` | Mode of a deleted or added file, or both modes when the mode changed. |
| `git_tag` | `name`, `target`, `tagger`, `time`, `message` | Name, tagged object, tagger after `.mailmap`, time and message. |
| `content` | `text`, `lines`, `total_lines` | Selected lines, their number and the number of lines of the file. |
| `content` | `range`, `truncated` | Lines returned and whether lines follow them, with `offset` or `limit`. |
| `content` | `etag`, `revision` | Identifier of the file object and of the commit it was read from. |
| `content` | `encoding`, `bom`, `line_ending`, `binary`, `size` | `utf-8`, `iso-8859-1` or `binary`; byte order mark; line endings; binary content; size in bytes. |

The report notes `excluded` with the number of changed files left out by
`exclude`.

## Effect

Category `source`, read only. Nothing in the repository changes.

## Examples

Show the last commit with its changes.

```nurt example
git.show "/home/user/repo" "HEAD"
```

```text expected
commit short_id: 4f2a9c1 subject: "fix(parser): accept empty input"
hunk path: src/parser.rs old_start: 41 old_len: 6 new_start: 41 new_len: 8 header: "fn parse(input: &str) -> Result<Ast> {"
```

Show the files changed by a merge against its first parent.

```nurt example
git.show "/home/user/repo" "HEAD" stat: true
```

Show a release tag and the commit it tags, without lock files.

```nurt example
git.show "/home/user/repo" "v1.4.0" exclude: ["*.lock"]
```

Read lines 100 to 149 of a file as it was two commits ago.

```nurt example
git.show "/home/user/repo" "HEAD~2" file: "/home/user/repo/src/main.rs" offset: 100 limit: 50
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `repo` is not inside a repository, or a path is outside it (problem `path_outside_repo`). | Give a path inside the working tree |
| `E_GIT_REVISION` | A revision is unknown, ambiguous, a range (`range`), or names a tree or a file (`not_a_commit`). | Give a commit or a tag; read files with `file` |
| `E_NOT_FOUND` | `file` does not exist at the revision. | Find the commits of the file with `git.log` |
| `E_INVALID_VALUE` | `file` names a directory or a submodule, `file` with `revs`, `paths` or `stat`, `offset` or `limit` without `file`, or `offset: 0`. | Remove the conflicting argument |
| `E_TYPE` | `exclude` is not a list of patterns. | Give glob patterns |
| `E_IO` | An object of the repository cannot be read. | Check the repository with `git.status` |
