---
group: git
summary: Read and change Git repositories in process, without an external program.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git

## Description

The commands of the `git` group read and change Git repositories in the
console process. No `git` program, credential helper, SSH client or other
program named by the repository configuration is started. Repository hooks
and filter drivers are the only programs of the user that these commands
run, and they run under the rules of `run`.

Every change of the working tree and of the repository directory is one
operation of the journal and can be undone with `undo`. Objects added to the
object store are kept by `undo`; `git gc` removes those that become
unreachable.

## Repository

The first parameter of every command is `repo: path`. It accepts the root of
a working tree, any directory below it, a linked working tree, a `.git`
directory or file, and a bare repository. The search for the repository
stops below every directory listed in `GIT_CEILING_DIRECTORIES`.

A repository owned by another account is opened only when `safe.directory`
lists it, as in `git`; the error offers the entry to add. A repository that
uses `extensions.refStorage=reftable` or an extension that is not
implemented fails with `E_GIT_UNSUPPORTED` before anything is read.
`core.fsmonitor` is not used: the files are checked directly and the report
notes `fsmonitor_ignored`.

## Paths

Paths in parameters are relative to the root of the working tree, whatever
the current directory is. An absolute path below the root is accepted; a path
outside it fails with `E_INVALID_VALUE` (problem `outside_repository`).

| Form | Selects |
|---|---|
| `src` | the file `src`, or every path below the directory `src` |
| `src/*.rs` | paths matching the glob; `*` also matches `/` |
| `.` | every path |

`exclude: list<glob>` removes paths from the selection with the syntax of
`.gitignore`: a pattern without `/` matches a name at any depth, a pattern
with `/` is anchored at the root, `**` matches any number of directories, and
an excluded directory excludes everything below it.

## Configuration

The configuration is read in the order of `git`; a later layer overrides an
earlier one.

| Layer | Source |
|---|---|
| system | `GIT_CONFIG_SYSTEM`, or the system file of the `git` distribution (`/etc/gitconfig`; on Windows `C:/Program Files/Git/etc/gitconfig`); none when `GIT_CONFIG_NOSYSTEM` is true |
| global | `GIT_CONFIG_GLOBAL`, or `$XDG_CONFIG_HOME/git/config` followed by `~/.gitconfig` |
| repository | `config` of the repository directory |
| working tree | `config.worktree`, when `extensions.worktreeConfig` is set |
| environment | `GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>` |

`include` and `includeIf` are followed. Environment variables are those of
the console session. A leading `~` in `core.excludesFile`,
`core.attributesFile`, `core.hooksPath` and `commit.template` is the home
directory of the session.

## Content filters

Content is converted between the working tree and the object store as `git`
converts it: `core.autocrlf`, `core.eol`, and the attributes `text`, `eol`,
`crlf`, `binary`, `ident` and `working-tree-encoding`.

A driver named by the attribute `filter` and configured as
`filter.<name>.clean`, `filter.<name>.smudge` or `filter.<name>.process` runs
as a program of the user through `sh -c`, with `%f` replaced by the quoted
path. When no program service is available, or a driver that is not
`filter.<name>.required` fails, the content passes unfiltered and the report
notes `filter_not_applied` with the number of files. A failing required
driver fails the command with `E_EXTERNAL_FAILED`. A Git LFS pointer file
that is not converted stays a pointer file in the working tree.

## Revisions

Revisions follow the syntax of `git rev-parse`.

| Form | Meaning |
|---|---|
| `main`, `v1.0`, `origin/main` | branch, tag or remote-tracking branch |
| `3f2a9c1` | abbreviated object identifier |
| `HEAD~2`, `HEAD^2` | ancestor, parent |
| `main@{1}`, `@{upstream}`, `@{push}` | reflog entry, upstream, push destination |
| `:/fix parser` | newest commit whose message matches |
| `HEAD:src/main.rs` | object at a path of a tree |
| `v1.0^{}`, `HEAD^{tree}` | peeled object |

A revision that cannot be resolved fails with `E_GIT_REVISION`. Its problem
is `not_found`, with the references of nearest name as candidates,
`ambiguous`, with the matching objects as candidates, `invalid_syntax`, or
`not_a_commit` where a commit is required. Abbreviated identifiers in
results have the length of `core.abbrev` and are extended until they are
unique.

## Remote addresses

| Address | Transport |
|---|---|
| `https://host/path`, `http://host/path` | smart HTTP |
| `ssh://user@host/path`, `user@host:path` | SSH |
| `file:///path`, `/path`, `../path` | local repository, served in process |

`url.<base>.insteadOf` rewrites addresses with the longest matching prefix;
for a push, `url.<base>.pushInsteadOf` takes precedence. `git://` and
`ext::` addresses fail with `E_GIT_UNSUPPORTED`.

## Locks

A file of the repository directory is not changed while its lock file
`<file>.lock` exists, which means that a `git` process is changing it or
ended without removing the lock. The command fails with `E_GIT_LOCKED` and
offers the removal of the lock as a fix.

## Errors

| Code | Problem | Cause |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `not_repository`, `path_not_found`, `unsafe_repository` | the path is not in a repository, does not exist, or belongs to another account |
| `E_GIT_UNSUPPORTED` | `unsupported_feature`, `unsupported_transport` | repository extension or address scheme without implementation |
| `E_GIT_REVISION` | `not_found`, `ambiguous`, `invalid_syntax`, `not_a_commit` | revision that cannot be resolved |
| `E_GIT_LOCKED` | `lock_exists` | lock file of `git` present |
| `E_INVALID_VALUE` | `outside_repository`, `invalid_url` | path outside the working tree, address that cannot be parsed |
| `E_EXTERNAL_FAILED` | `filter_failed` | required filter driver failed |
| `E_STALE` | | a file changed between reading and writing |
| `E_IO` | `corrupt_repository`, `repository_error`, `filter_failed` | repository data that cannot be read or converted |

## Commands

| Command | Purpose |
|---|---|
| [`git.status`](git.status.md) | state of the working tree and the index |
| [`git.diff`](git.diff.md) | differences with hunk identifiers |
| [`git.log`](git.log.md) | history |
| [`git.show`](git.show.md) | commit, tag or file at a revision |
| [`git.blame`](git.blame.md) | origin of lines |
| [`git.branch`](git.branch.md) | branches |
| [`git.tag`](git.tag.md) | tags |
| [`git.stage`](git.stage.md) | index: files, hunks and lines |
| [`git.commit`](git.commit.md) | commits |
| [`git.review`](git.review.md) | checks of changes before a commit |
| [`git.checkpoint`](git.checkpoint.md) | snapshots of the working tree |
| [`git.worktree`](git.worktree.md) | linked working trees |
| [`git.clone`](git.clone.md) | copy of a repository |
| [`git.fetch`](git.fetch.md) | download from a remote |
| [`git.switch`](git.switch.md) | change of the current branch |
| [`git.restore`](git.restore.md) | restore files from a revision or the index |
| [`git.reset`](git.reset.md) | move the current branch |
| [`git.stash`](git.stash.md) | set changes aside |
| [`git.merge`](git.merge.md) | merge histories |
| [`git.rebase`](git.rebase.md) | replay commits |
| [`git.push`](git.push.md) | upload to a remote |
| [`git.pull`](git.pull.md) | fetch and integrate |

## Examples

Status of the repository that contains a subdirectory:

```nurt
git.status "/work/app/src/net"
```

Previous position of a branch from its reflog:

```nurt
git.log "/work/app" revision: "main@{1}" count: 1
```

Staging a directory without lock files:

```nurt
git.stage "/work/app" action: add paths: ["docs"] exclude: ["*.lock"]
```

History of the upstream of the current branch:

```nurt
git.log "/work/app" revision: "@{upstream}" count: 5
```
