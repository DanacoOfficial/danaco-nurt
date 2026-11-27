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
git.log <repo> [name: value]...
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
`.mailmap`. The field `trailers` of a commit record maps each trailer token
to its value; a token that occurs several times keeps every value, one per
line in message order, as `git interpret-trailers` lists them, in `git.log` and
in every other command that returns commit records. `since` stops the traversal at the first commit older than the
instant; `until` hides commits newer than it. Both compare the committer time
in whole seconds and include the bound.

`group_by` replaces the commits by `map` records with the number of selected
commits per Conventional Commits type (`other` for other subjects), per author
or per author day, most frequent first.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `count` | `int` | `20` | Maximum number of commits. |
| `skip` | `int` | `0` | Number of commits to skip. |
| `path` | `list<path>` | none | Only commits that change these files or directories. |
| `revision` | `text` | `"HEAD"` | Revision or range to list. |
| `author` | `text` | none | Only commits whose author matches this text. |
| `since` | `instant` | none | Only commits at or after this time. |
| `until` | `instant` | none | Only commits before this time. |
| `message` | `regex` | none | Only commits whose message matches this pattern. |
| `pickaxe` | `text \| regex` | none | Only commits that add or remove this text or pattern. |
| `follow` | `bool` | `false` | Follow a single file across renames. |
| `symbol` | `text` | none | Only commits that change the lines of this code element. |
| `stat` | `bool` | `false` | Include changed files with added and removed lines. |
| `group_by` | `enum(type\|author\|day)` | none | Group commits by conventional commit type, author or day. |
| `diff_regex` | `regex` | none | Only commits whose changed lines match this pattern. |
| `exclude` | `list<glob>` | `[]` | Paths to leave out, also from the pickaxe and diff_regex filters. |
| `find_renames` | `bool` | `false` | Detect renamed files when limiting the log to paths. |
| `find_copies` | `bool` | `false` | Detect copied files when limiting the log to paths. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `commit`, `group` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |
| External use | `user_program`: `gpg`; Runs gpg (gpg.openpgp.program or gpg.program) to verify OpenPGP signatures when the signature field of the commits is selected. |

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

A response that leaves records out ends with the hint "more: next" and carries the field
`continuation`; the command `next` returns the next part without running the
command again.

```text
matched 900  shown 50  more: next
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
| `E_SYMBOL_NOT_FOUND` | The element is not defined, or defined several times; the candidates are listed. | Use a qualified name from the candidates |
| `E_SYMBOL_AMBIGUOUS` | The element is not defined, or defined several times; the candidates are listed. | Use a qualified name from the candidates |
| `E_NOT_FOUND` | The file of `symbol` does not exist at the listed revision. | Give an existing file |
| `E_IO` | An object of the repository cannot be read. | Check the repository with `git.status` |

## Related

`git.show`, `git.blame`, `git.diff`, `git.branch`, `git.tag`.
