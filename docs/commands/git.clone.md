---
command: git.clone
group: git
summary: Clone a repository over HTTPS or SSH into a new directory.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.clone

## Synopsis

```text
git.clone <url> <to> [name: value]...
```

## Description

Clones a repository into a directory that does not exist, as `git clone`
does.

`url` is an address reached through smart HTTP (`http://`, `https://`), SSH
(`ssh://`, `user@host:path`), a `file://` address or an absolute local path.
A relative local path is refused. `to` must not exist and its parent
directory must.

The result has the layout that `git clone` creates:

- the remote `origin` with the address and, in a repository with a working
  tree, the refspec `+refs/heads/*:refs/remotes/origin/*`;
- the branches of the remote as `refs/remotes/origin/*`, and the tags, in
  `packed-refs`;
- `refs/remotes/origin/HEAD` naming the default branch of the remote;
- the local branch `branch`, or the default branch of the remote, with
  `branch.<name>.remote` and `branch.<name>.merge`, checked out into the
  working tree and the index;
- the reflog message `clone: from <address>` for `HEAD`, the branch and
  `refs/remotes/origin/HEAD`.

A remote whose `HEAD` is detached gives a detached `HEAD`. A remote without
commits gives an unborn branch named after its default branch.

With `depth` only the chosen branch is fetched, its history is limited to
that many commits, `shallow` records the boundary, the refspec names the
single branch, and only the tags that point into the fetched history are
stored. With `bare: true` the repository has no working tree, the branches
of the remote become local branches, no refspec and no reflog are written.

The address stored in `remote.origin.url` has no password. The credentials
for HTTP are taken from the address, from the variables `NURT_GIT_TOKEN_<HOST>`
and `NURT_GIT_TOKEN`, from the variables of hosting providers or from the
credential stores; credential helpers that are programs are not run.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `url` | `text` | required | Address of the repository. |
| `to` | `path` | required | Directory to create. |
| `depth` | `int` | none | Limit the cloned history to this many commits. |
| `branch` | `text` | none | Branch to check out instead of the default branch. |
| `bare` | `bool` | `false` | Create a repository without a working tree. |
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
| Reversibility conditions | The new directory is removed by undo; the network transfer cannot be reversed. |

## Examples

Clones the default branch of a repository.

```nurt example
git.clone "https://example.com/org/app.git" "/home/user/app"
```

```text expected
effect  git.clone  ok  /home/user/app  state: completed
```

Clones only the last commit of branch `release`.

```nurt example
git.clone "https://example.com/org/app.git" "/home/user/app" branch: "release" depth: 1
```

Creates a bare repository from a local path.

```nurt example
git.clone "/srv/git/app.git" "/home/user/app.git" bare: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_EXISTS` | `to` exists (`destination_exists`). | Pass a directory that does not exist. |
| `E_NOT_FOUND` | The parent directory of `to` does not exist (`parent_missing`). | Create the parent directory first. |
| `E_INVALID_VALUE` | `depth` is 0, or `url` is empty or a relative local path. | Pass `depth` of 1 or more and an absolute path or a `file://` address. |
| `E_GIT_REF` | `branch` is not a branch of the remote (`remote_branch_not_found`). | Pass one of the branches listed in the error, or omit `branch`. |
| `E_GIT_AUTH` | The remote requires credentials that are missing or were refused. | Set `NURT_GIT_TOKEN_<HOST>` and use an `https://` address. |
| `E_GIT_HOST_KEY` | The SSH host key is unknown or changed. | Pass `accept_host_key` with the fingerprint of the host. |
| `E_GIT_PROTOCOL` | The remote answered outside the protocol. | Check that the address names a Git repository. |
| `E_NETWORK` | The connection failed; the problem is `dns`, `connect`, `tls`, `timeout`, `proxy` or `http_status`. | Check the address, the proxy settings and the network. |
| `E_CANCELLED` | The transfer was interrupted; the new directory was removed. | Repeat the call. |
| `E_TIMEOUT` | The transfer was interrupted; the new directory was removed. | Repeat the call. |

## Related

`git.fetch`, `git.switch`, `git.worktree`, `git.status`, `undo`.
