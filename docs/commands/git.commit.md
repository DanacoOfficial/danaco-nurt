---
command: git.commit
group: git
summary: Record a commit and report its branch and files.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.commit

## Synopsis

```text
git.commit <repo> message: <text> [name: value]...
```

## Description

Records a commit on the branch `HEAD` points to, or on `HEAD` itself when it
is detached, and reports the new commit with its files.

Exactly one of `staged`, `all` and `paths` selects the content:

- `staged: true` commits the index as it is. While a merge is in progress
  (`MERGE_HEAD` exists), the commit concludes the merge: its parents are
  `HEAD` and the commits of `MERGE_HEAD`, the files `MERGE_HEAD`,
  `MERGE_MSG`, `MERGE_MODE` and `AUTO_MERGE` are removed and the reflog entry
  reads `commit (merge): <subject>`.
- `all: true` stages every change of the working tree first: modified and
  deleted tracked files and untracked files that are not ignored. The index
  is written with the staged content.
- `paths` commits `HEAD` with the working tree content of the given files
  and directories, as `git commit --only` does. The index entries of these
  paths are updated; other staged changes stay in the index and out of the
  commit. A path that matches no file of the working tree or the index is an
  error.

`amend: true` without a selection replaces only the message of the last
commit: the tree and the parents stay, and the author and author time are
kept. With a selection, `amend` replaces the last commit by a commit of the
selected content with the same parents. Amending is refused while a merge is
in progress.

The message is completed with a line feed, followed by
`Signed-off-by: <committer>` with `sign_off` and by `trailers`, which join an
existing trailer block or start one after a blank line; a trailer equal to
the last trailer of the block is not repeated. The hooks `prepare-commit-msg`
and `commit-msg` may change the message; the result is cleaned up according
to `commit.cleanup` (`strip` also removes lines starting with
`core.commentChar`). The author comes from `author`, `GIT_AUTHOR_NAME`,
`GIT_AUTHOR_EMAIL`, `author.*`, `user.*` and `EMAIL`; the committer from
`GIT_COMMITTER_*`, `committer.*`, `user.*` and `EMAIL`, with the parts of
`author` as the last resort. Dates come from `GIT_AUTHOR_DATE` and
`GIT_COMMITTER_DATE`, otherwise from the clock with the host time offset.
`i18n.commitEncoding` other than UTF-8 is recorded in the `encoding` header.
With `sign: true`, or without `sign` when `commit.gpgSign` is set, the
commit is signed with the key of `user.signingKey` in the format of
`gpg.format` (`openpgp` or `ssh`). An OpenPGP signature is made by GnuPG: the
program named by `gpg.openpgp.program` or `gpg.program` (`gpg` by default) is
started through the program service, as `run` starts programs, with the
arguments of `git commit -S` and the commit on its standard input. The key of
`git.signing.key_file` is imported into a temporary GnuPG home directory that
is removed, with its agent, after signing; the passphrase comes from the file
named by `NURT_GPG_PASSPHRASE_FILE`. A missing or failing program refuses the
commit with `E_GIT_SIGN` and the problem `program_unavailable` or
`program_failed`, and nothing is written. An SSH signature is made in the
process, without running `ssh-keygen`; `gpg.ssh.program` is not run.
`sign: false` leaves the commit unsigned whatever `commit.gpgSign` says.

A commit whose tree equals the tree of its first parent (the empty tree for
the first commit of a branch) is refused unless `allow_empty` is true or the
commit concludes a merge. `exclude` leaves matching paths out: their changes
are not staged and the commit records their version from `HEAD`; the report
counts them.

The hooks `pre-commit`, `prepare-commit-msg`, `commit-msg` and `post-commit`
of `core.hooksPath`, or of `hooks/` in the repository directory, run as user
programs in the root of the working tree, with the limit `git.hooks.timeout`.
They receive a copy of the index (`GIT_INDEX_FILE`) and of the message in a
scratch directory outside the repository; content a `pre-commit` hook stages
into the copy becomes part of the commit. A failing `post-commit` hook does
not change the result.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `message` | `text` | required | Commit message. |
| `paths` | `list<path>` | none | Files to stage before committing. |
| `all` | `bool` | `false` | Stage all changes to tracked files before committing. |
| `staged` | `bool` | `false` | Commit exactly the content of the index. |
| `amend` | `bool` | `false` | Replace the last commit. |
| `trailers` | `list<text>` | `[]` | Trailers appended to the message, such as Co-Authored-By lines. |
| `sign_off` | `bool` | `false` | Append a Signed-off-by trailer. |
| `author` | `text` | the repository configuration | Author as Name <email>. |
| `allow_empty` | `bool` | `false` | Allow a commit without changes. |
| `hooks` | `bool` | `true` | Run the pre-commit, prepare-commit-msg, commit-msg and post-commit hooks of the repository as user programs. |
| `sign` | `bool` | the setting commit.gpgSign of the repository configuration | Sign the commit with the signing key of the repository configuration. |
| `exclude` | `list<glob>` | `[]` | Paths to leave out of the commit; the report counts the excluded files. |
| `expect_state` | `text` | none | State token from an earlier read; nothing changes when the repository state differs. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect`, `commit` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |
| Reversibility conditions | Undo moves the branch reference back; the working tree is not changed and written objects remain in the repository. |
| External use | `user_program`; used when `hooks` is given; Runs the pre-commit, prepare-commit-msg, commit-msg and post-commit hooks of the repository as user programs. |

## Examples

Commit every change of the working tree with a sign-off.

```nurt example
git.commit "/home/user/repo" message: "feat: add the parser" all: true sign_off: true
```

```text expected
effect kind: git.commit status: ok
commit short_id: 3f2a9c1 subject: "feat: add the parser" refs: [main]
```

Commit two files only, leaving other staged changes in the index.

```nurt example
git.commit "/home/user/repo" message: "fix: handle empty input" paths: ["src/parse.rs", "tests/parse.rs"]
```

Add a co-author trailer to the staged changes and skip the hooks.

```nurt example
git.commit "/home/user/repo" message: "docs: describe options" staged: true trailers: ["Co-Authored-By: Ada Lovelace <ada@example.com>"] hooks: false
```

Reword the last commit.

```nurt example
git.commit "/home/user/repo" message: "fix: handle empty input files" amend: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_PARAM_CONFLICT` | None or more than one of `staged`, `all`, `paths` (problems `missing_selection`, `selection_conflict`), `amend` or `paths` during a merge. | Give one selection; conclude the merge with `staged: true` |
| `E_GIT_NOTHING_TO_COMMIT` | The commit would record no change (problem `nothing_to_commit`, `amend_empty`). | Stage changes, use `all: true`, or `allow_empty: true` |
| `E_GIT_IN_PROGRESS` | The index has unresolved conflicts (problem `unmerged_paths`). | Resolve and stage the files with `git.stage` |
| `E_CONFIG` | No author or committer identity (problem `missing_identity`), or `commit.cleanup` has an invalid value. | Set `user.name` and `user.email`, or give `author` |
| `E_INVALID_VALUE` | `author` is not `Name <email>`, a trailer is malformed, a date variable is invalid, or the message is empty after cleanup. | Correct the value |
| `E_NOT_FOUND` | A path of `paths` matches no file (problem `path_not_found`). | Check the spelling with `git.status` |
| `E_GIT_HOOK` | `pre-commit`, `prepare-commit-msg` or `commit-msg` failed; the error carries the last lines of its output. | Correct the reported problem, or `hooks: false` |
| `E_TIMEOUT` | A hook exceeded `git.hooks.timeout`. | Raise the limit or skip the hooks |
| `E_GIT_SIGN` | The commit cannot be signed (`no_key`, `agent_unavailable`, `key_locked`, `unsupported_format`). | Configure `user.signingKey` or start the key agent |
| `E_STALE` | `expect_state` differs from the repository state (problem `state_changed`), or a file changed during the commit. | Read the state again and repeat the call |
| `E_GIT_REF` | The branch moved after the plan. | Repeat the call |
| `E_GIT_LOCKED` | A lock file of `git` exists. | Wait for the other `git` process or remove the stale lock |
| `E_GIT_UNSUPPORTED` | `all` or `paths` in a bare repository (problem `bare_repository`). | Use `staged: true` |

## Related

`git.status`, `git.stage`, `git.review`, `git.log`, `undo`.
