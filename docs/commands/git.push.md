---
command: git.push
group: git
summary: Update references on a remote with local commits.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.push

## Synopsis

```text
git.push <repo> branch: <text> [name: value]...
```

## Description

Sends the commits needed by the remote and updates its references through
the send-pack protocol (protocol version 0 with `report-status-v2`,
`side-band-64k`, `atomic`, `push-options`, `ofs-delta`, `quiet`, `agent` and
`object-format` when the remote announces them).

`branch` is a refspec:

- `main` pushes the local branch or tag to the remote reference of the same
  name; `HEAD` pushes the current branch;
- `src:dst` pushes the revision `src` to the remote reference `dst`; an
  unqualified `dst` is completed with `refs/heads/` or `refs/tags/` as the
  remote or the source requires;
- `:dst` deletes the remote reference;
- a leading `+` forces the update of that refspec.

Before the request every update is classified as `git push` does:
`up_to_date` updates are not sent; a new reference is `new`; an update whose
old value is an ancestor of the new one is `updated`; any other update is
rejected as `non_fast_forward` (the old commit is present locally) or
`fetch_first` (it is not) unless forced. A remote tag is replaced only by a
forced update (`already_exists`). With a lease the remote reference must
still have the expected value, otherwise the update is rejected as
`stale_info`.

The objects sent are those reachable from the new values and not reachable
from the references of the remote. Entries stored as deltas in local packs
are copied; no new deltas are computed.

Updates accepted by the remote move the remote-tracking references named by
`remote.<name>.fetch`; their reflog entries read `update by push`. With
`set_upstream` the configuration receives `branch.<name>.remote` and
`branch.<name>.merge`. The `pre-push` hook of the repository runs before the
request; a non-zero exit stops the push.

A remote on the local file system (`file://` URL or path) is served in the
process: the pack is stored in `objects/pack` of the target, the new tips are
checked for completeness, `receive.denyCurrentBranch` (default `refuse` for a
repository with a working tree), `receive.denyDeleteCurrent`,
`receive.denyNonFastForwards`, `receive.denyDeletes` and `receive.hideRefs`
apply, and the hooks `pre-receive`, `update` and `post-receive` of the target
run as user programs. The reflog entries of the target read `push`.

Other remotes are reached over smart HTTP (`http://`, `https://`) or SSH
(`ssh://`, `user@host:path`) after the `url.<base>.pushInsteadOf` and
`url.<base>.insteadOf` rewrites. The `http.*` settings of the repository
apply. When the server answers `401`, the credentials for the address are
taken from the variables `NURT_GIT_TOKEN_<HOST>` and `NURT_GIT_TOKEN`, from
the variables of hosting providers or from the credential stores, and the
request is repeated once; credentials are never sent over `http://`.
Credential helpers that are programs are not run. The `git://` and `ext::`
transports start programs and are not supported.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or a path inside it. |
| `branch` | `text` | required | Refspec as described above. |
| `remote` | `text` | `"origin"` | Configured remote, URL or path. `remote.<name>.pushurl` takes precedence over `remote.<name>.url`. |
| `force` | `bool \| enum(no\|lease\|yes)` | `false` | `true` and `lease` overwrite only while the remote reference has the value of its remote-tracking reference; `yes` overwrites unconditionally. |
| `force_with_lease` | `text` | none | Revision the remote reference must still have; implies a lease. |
| `tags` | `bool` | `false` | Also push every local tag. |
| `set_upstream` | `bool` | `false` | Record the pushed branch as the upstream of the local branch. |
| `delete` | `bool` | `false` | Delete the remote reference named by `branch`. |
| `accept_host_key` | `text` | none | SHA-256 fingerprint (SHA256:<base64>) of the SSH host key to accept when the host is not known. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect`, `git_ref_update` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `external` |
| Reversibility | `irreversible` |
| Implementation | `builtin` |

## Examples

Push the current branch and record it as upstream.

```nurt example
git.push "/home/user/repo" branch: "HEAD" set_upstream: true
```

```text expected
effect git.push state: completed upstream: origin/main
git_ref_update ref: refs/heads/main status: new remote: origin remote_ref: refs/heads/main
```

Replace a rewritten branch only while the remote still has the fetched
commit.

```nurt example
git.push "/home/user/repo" branch: "feature/login" force: lease
```

Push a commit to a differently named remote branch of a local mirror.

```nurt example
git.push "/home/user/repo" remote: "/srv/git/mirror.git" branch: "4f2a9c1:refs/heads/release"
```

Delete a remote branch.

```nurt example
git.push "/home/user/repo" branch: "old-topic" delete: true
```

Push a branch together with all tags.

```nurt example
git.push "/home/user/repo" branch: "main" tags: true
```

Push to a hosting service over HTTPS with a token from the environment.

```nurt example
git.push "/home/user/repo" remote: "https://git.example.com/team/app.git" branch: "main"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_PUSH_REJECTED` | No reference was updated; the details list every update with its reason (`non_fast_forward`, `fetch_first`, `stale_info`, `already_exists`, `checked_out`, `hook_declined`, `atomic_failed`, `deletion_prohibited`, `remote_ref_missing`). | `git.pull "/home/user/repo" remote: "origin"`, then push again |
| `E_GIT_HOOK` | The `pre-push` hook declined the push. | Correct the problem reported by the hook |
| `E_GIT_REVISION` | The source of the refspec does not resolve, or a revision has no destination. | Use `src:refs/heads/<name>` |
| `E_GIT_REF` | The refspec or a reference name is invalid. | Correct the name |
| `E_NOT_FOUND` | The remote is not configured. | Pass a URL or path, or configure `remote.<name>.url` |
| `E_GIT_NOT_REPOSITORY` | The repository or the local remote is not a Git repository. | Check the path and the remote URL |
| `E_GIT_UNSUPPORTED` | The address uses `git://` or `ext::`, or SSH in a build without SSH. | Use an `https://` or `ssh://` address |
| `E_GIT_AUTH` | The remote requires credentials that are missing, were refused, or would be sent over `http://`. | Set `NURT_GIT_TOKEN_<HOST>` and use an `https://` address |
| `E_NETWORK` | The remote cannot be reached or answered with an HTTP error. | Check the address, the proxy settings and the connection |
| `E_INVALID_VALUE` | The address or an `http.*` setting cannot be parsed. | Correct the remote URL or the setting named in the message |
| `E_GIT_PROTOCOL` | The remote broke the protocol or lacks a required capability. | Push without the option named in the message |
| `E_GIT_LOCKED` | A reference lock or `config.lock` exists. | Wait for the other process or remove the stale lock |
| `E_STALE` | A reference changed while the push ran. | Fetch and push again |

## Related

`git.pull`, `git.fetch`, `git.branch`, `undo`.

.
