---
command: git.pull
group: git
summary: Fetch from a remote and integrate the changes into the current branch.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/git.pull.md -->
<!-- Responsibility:   Documents git.pull. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# git.pull

## Synopsis

```text
git.pull <repo> [name: value]...
```

## Description

Fetches from a remote and integrates the fetched branch into the current
branch, as `git pull` does.

The remote is `remote`, otherwise `branch.<name>.remote` of the current
branch, otherwise `origin`. The branch to integrate is `branch`, otherwise
the upstream branch of the current branch (`branch.<name>.merge`). Without
`branch`, the remote must be the upstream remote of the current branch.

The integration follows `mode`. Without `mode` the repository configuration
decides: `branch.<name>.rebase`, then `pull.rebase` select a rebase
(`true`, `merges`, `interactive`) or a merge (`false`); `pull.ff` selects
the fast-forward policy (`true`, `false`, `only`). The steps are:

- When the current branch is contained in the fetched branch, the branch is
  fast-forwarded, also when a rebase is configured.
- When the fetched branch is contained in the current branch, nothing
  changes and the state is `up_to_date`.
- When the histories diverged, `mode: merge` creates a merge commit with the
  message `Merge branch '<branch>' of <url>`, followed by ` into <current>`
  unless the current branch matches `merge.suppressDest` (by default `main`
  and `master`); `mode: rebase` replays the local commits onto the fetched
  branch, starting after the fork point recorded in the reflog of the
  remote-tracking branch; `mode: fast_forward_only` and `pull.ff=only`
  refuse.
- When neither `mode` nor the configuration chooses, diverged histories are
  refused with `E_GIT_NON_FAST_FORWARD` (problem `diverged`), as in Git 2.53,
  and the error offers `mode: merge` and `mode: rebase`.
- On a branch without commits, the fetched commit is checked out and the
  branch is created with the reflog message `initial pull`.

Checks that do not need the remote run before the transfer and change
nothing: unmerged files, a merge or rebase in progress, and, for a rebase
without `rebase.autoStash`, unstaged or staged changes. Local changes and
untracked files in the way of a fast-forward or a merge are found after the
transfer; then, as in Git, the fetched references and `FETCH_HEAD` are
updated and the working tree and the current branch stay unchanged.

A merge with conflicts stops as `git.merge` does: the working tree holds the
conflict markers, the index holds the conflict stages, and `MERGE_HEAD`,
`MERGE_MSG`, `MERGE_MODE`, `AUTO_MERGE` and `ORIG_HEAD` are written. The
conflict markers name the merged commit by its full identifier, as Git does
for a pull.

Programs named by the configuration (credential helpers, `core.sshCommand`,
content filters) do not run unless the corresponding setting of the console
allows them; skipped programs are reported as notable entries.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `remote` | `text` | `"origin"` | Remote to pull from. |
| `branch` | `text` | the upstream of the current branch | Remote branch to integrate. |
| `mode` | `enum(merge\|rebase\|fast_forward_only)` | the setting of the repository configuration | How the changes are integrated. |
| `accept_host_key` | `text` | none | SHA-256 fingerprint (SHA256:<base64>) of the SSH host key to accept when the host is not known. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect`, `git_ref_update`, `git_conflict` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `external` |
| Reversibility | `irreversible` |
| Implementation | `builtin` |
| Reversibility conditions | Local changes to the working tree are kept in the journal; the network transfer cannot be reversed. |

## Examples

Pulls the upstream branch of the current branch with the configured mode.

```nurt example
git.pull "/home/user/app"
```

```text expected
effect  git.pull  ok  remote:origin  state: fast_forward
```

Pulls branch `release` of remote `upstream` and creates a merge commit when
the histories diverged.

```nurt example
git.pull "/home/user/app" remote: "upstream" branch: "release" mode: merge
```

Replays the local commits onto the upstream branch.

```nurt example
git.pull "/home/user/app" mode: rebase
```

Updates the current branch only when no merge is needed, and accepts the
host key of a new SSH host.

```nurt example
git.pull "/home/user/app" mode: fast_forward_only accept_host_key: "SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `repo` is not inside a repository. | Give the path of a repository. |
| `E_GIT_IN_PROGRESS` | Unmerged files, or a merge or rebase in progress. | `git.merge "<repo>" action: abort`, `git.rebase "<repo>" action: continue` |
| `E_GIT_DIRTY` | Uncommitted changes block a rebase (before the transfer), or local changes would be overwritten (after the transfer, `effect_applied`). | `git.stash "<repo>" action: push` |
| `E_GIT_NON_FAST_FORWARD` | The histories diverged without a chosen mode (`diverged`), or only a fast-forward is allowed (`not_fast_forward`). | `git.pull "<repo>" mode: merge` or `mode: rebase` |
| `E_GIT_REF` | No upstream branch (`no_upstream`), detached `HEAD`, a remote other than the upstream remote without `branch`, or the branch does not exist on the remote. | `git.pull "<repo>" remote: "origin" branch: "<name>"` |
| `E_GIT_CONFLICT` | The merge stopped with conflicts; `effect_applied` is true. | Resolve, then `git.merge "<repo>" action: continue` |
| `E_GIT_UNSUPPORTED` | Invalid `pull.rebase` or `pull.ff` value, a bare repository, several branches to merge at once, or a rebase that the installation does not provide. | Correct the configuration or give `mode`. |
| `E_GIT_LOCKED` | A lock file of the index or of a reference exists. | Remove the stale `.lock` file after checking that no Git process runs. |
| `E_NETWORK` | The transfer failed; nothing local changed. | See `git.fetch`. |
| `E_GIT_AUTH` | The transfer failed; nothing local changed. | See `git.fetch`. |
| `E_GIT_HOST_KEY` | The transfer failed; nothing local changed. | See `git.fetch`. |
| `E_GIT_PROTOCOL` | The transfer failed; nothing local changed. | See `git.fetch`. |

Errors after the transfer carry `effect_applied: true` and list the updated
remote-tracking references in `details.ref_updates`.

## Related

`git.fetch`, `git.merge`, `git.rebase`, `git.push`, `git.status`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
