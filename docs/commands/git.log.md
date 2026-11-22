---
command: git.log
group: git
summary: List commits of a revision, optionally limited to files, authors, dates or content.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.log

## Synopsis

```text
git.log <repo: path> [revision: <text>] [count: <int>] [skip: <int>] [path: <list<path>>] [author: <text>] [since: <instant>] [until: <instant>] [message: <regex>] [pickaxe: <text|regex>] [diff_regex: <regex>] [follow: <bool>] [symbol: <text>] [stat: <bool>] [group_by: type|author|day] [exclude: <list<glob>>]
```

## Description

Lists commits in the default order of `git log`: newest first by committer
time, without topological sorting; commits with equal times appear in the
order in which they were reached. `revision` accepts a single revision, a
range `A..B` (commits reachable from `B` and not from `A`), a symmetric range
`A...B`, an exclusion `^A`, the forms `A^@` and `A^!`, and several of these
separated by spaces. A repository without commits has an empty history.

With `path`, history is simplified as `git log -- <path>` simplifies it: a
commit is listed when it changes one of the paths, and a merge that equals
one of its parents within the paths is followed through that parent only. A
path names a file or a directory with everything below it.

`follow` lists the history of one file across renames and copies: when the
file appears in a commit and has a similar source in the parent, the source
is followed from there on. `symbol` lists the commits that changed the lines
of one code element of the file, as `git log -L` does; the lines are found
with the code service at the first listed revision and carried to older
commits through the differences.

`pickaxe` with a text keeps commits that change the number of occurrences of
the text in a file (`git log -S`); with a regular expression it keeps commits
that change the number of matches (`-S --pickaxe-regex`). `diff_regex` keeps
commits with an added or removed line that matches (`git log -G`). The two
cannot be combined. Merges have no changes of their own, so `follow`,
`pickaxe` and `diff_regex` never select them; binary files and submodules are
not searched.

`author` is a regular expression matched against `Name <email>`, and
`message` against each line of the message. Identities are mapped through
`.mailmap`. `since` stops the traversal at the first commit older than the
instant; `until` hides commits newer than it. Both compare the committer time
in whole seconds and include the bound.

`group_by` replaces the commits by `map` records with the number of selected
commits per Conventional Commits type (`other` for other subjects), per author
or per author day, most frequent first.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `revision` | `text` | `"HEAD"` | Revision or range to list. |
| `count` | `int` | `20` | Largest number of commits listed. |
| `skip` | `int` | `0` | Number of selected commits left out before the first listed one. |
| `path` | `list<path>` | none | Only commits that change these files or directories. |
| `author` | `text` | none | Regular expression matched against `Name <email>` of the author. |
| `since` | `instant` | none | Only commits at or after this committer time. |
| `until` | `instant` | none | Only commits at or before this committer time. |
| `message` | `regex` | none | Only commits with a message line that matches. |
| `pickaxe` | `text \| regex` | none | Only commits that change the number of occurrences or matches in a file. |
| `diff_regex` | `regex` | none | Only commits with an added or removed line that matches. |
| `follow` | `bool` | `false` | Follow the single file in `path` across renames and copies. |
| `symbol` | `text` | none | Only commits that change the lines of this element of the single file in `path`. |
| `stat` | `bool` | `false` | Add the changed files with added and removed lines. |
| `group_by` | `enum(type\|author\|day)` | none | Count commits per Conventional Commits type, author or day. |
| `exclude` | `list<glob>` | `[]` | Paths left out of `stat`, `pickaxe` and `diff_regex`; a pattern without `/` matches the file name in any directory. |

## Input and output

No input. The output is one `commit` record per listed commit, or one `map`
record `{key, count}` per group with `group_by`.

| Field | Type | Description |
|---|---|---|
| `id`, `short_id` | `text` | Identifier and its shortest unique abbreviation, at least 7 digits. |
| `author`, `author_email` | `text` | Author after `.mailmap`. |
| `time` | `instant` | Author time. |
| `committer`, `committer_email` | `text` | Committer after `.mailmap`. |
| `commit_time` | `instant` | Committer time. |
| `subject`, `message` | `text` | First paragraph line and the whole message without the final line break. |
| `parents` | `list<text>` | Parent identifiers in recorded order. |
| `tree` | `text` | Root tree. |
| `trailers` | `map` | Trailers of the last paragraph; repeated keys are joined with `, `. |
| `refs` | `list<text>` | Names pointing at the commit as `git log --decorate` writes them, read only when used. |
| `signature` | `record` | Format of the signature, only when the field is selected by name; signatures are not verified (`status: unsupported`). |
| `files`, `stats` | `list<record>`, `record` | Changed files and totals with `stat`; binary files count no lines. |

## Effect

Category `source`, read only. Nothing in the repository changes.

## Examples

List the last five commits.

```nurt example
git.log "/home/user/repo" count: 5
```

```text expected
commit short_id: 4f2a9c1 subject: "fix(parser): accept empty input" author: "Ann Lee"
commit short_id: 9b07d3e subject: "feat: add the export command" author: "Ann Lee"
```

List the commits of a branch that are not in `main`, with changed files.

```nurt example
git.log "/home/user/repo" revision: "main..topic" stat: true
```

Follow one file across renames and keep the commits that added or removed a
call.

```nurt example
git.log "/home/user/repo" path: "/home/user/repo/src/io.rs" follow: true pickaxe: "read_all("
```

Count the commits of the last 30 days per Conventional Commits type.

```nurt example
git.log "/home/user/repo" since: now - 30d count: 1000 group_by: type
```

List the history of one function.

```nurt example
git.log "/home/user/repo" path: "/home/user/repo/src/lib.rs" symbol: "parse_header"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `repo` is not inside a repository, or a path is outside it (problem `path_outside_repo`). | Give a path inside the working tree |
| `E_GIT_REVISION` | A revision is unknown (`not_found`), ambiguous (`ambiguous`), malformed (`invalid_syntax`) or names no commit (`not_a_commit`). | List branches with `git.branch` |
| `E_REGEX` | `author`, `message`, `pickaxe` or `diff_regex` is not a valid pattern. | Correct the pattern |
| `E_MISSING_PARAM` | `follow` or `symbol` without a file in `path`. | Give one file in `path` |
| `E_INVALID_VALUE` | More than one file with `follow` or `symbol`, `symbol` with `follow` or `stat`, `pickaxe` with `diff_regex`, an empty `pickaxe`, a negative `count` or `skip`, or an unknown `group_by`. | Remove one of the arguments |
| `E_UNSUPPORTED` | `symbol` on a host without the code service. | Use `path` without `symbol` |
| `E_UNSUPPORTED_LANGUAGE` | `symbol` in a file without a grammar. | Use `path` without `symbol` |
| `E_SYMBOL_NOT_FOUND`, `E_SYMBOL_AMBIGUOUS` | The element is not defined, or defined several times; the candidates are listed. | Use a qualified name from the candidates |
| `E_NOT_FOUND` | The file of `symbol` does not exist at the listed revision. | Give an existing file |
| `E_IO` | An object of the repository cannot be read. | Check the repository with `git.status` |
