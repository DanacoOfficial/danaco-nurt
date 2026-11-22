---
command: git.fetch
group: git
summary: Fetch objects and references from a remote over HTTPS or SSH.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.fetch

## Synopsis

```text
git.fetch <repo> [name: value]...
```

## Description

Fetches objects and references from a remote into the repository, as
`git fetch` does.

`remote` is the name of a configured remote or an address. A name selects the
refspecs of `remote.<name>.fetch`; an address fetches the remote `HEAD` into
`FETCH_HEAD` only. The remote is reached through a local path or `file://`
address, smart HTTP (`http://`, `https://`) or SSH (`ssh://`,
`user@host:path`) after the `url.<base>.insteadOf` rewrites. Protocol version
2 is used unless `protocol.version` is `0` or `1`.

For every remote reference selected by a refspec, the local reference is
created, fast-forwarded, or, when the refspec starts with `+`, moved
without a fast-forward check. A reference that would move without force is
rejected and reported with the status `rejected`. The branch checked out in
the working tree is never updated through a refspec.

Tags that point into the fetched history are fetched unless
`remote.<name>.tagOpt` is `--no-tags`. With `tags: true` every tag of the
remote is fetched. With `prune: true`, or `fetch.prune` or
`remote.<name>.prune` set, the local references that match the right-hand
side of a refspec and no longer exist on the remote are deleted.

`depth` limits the fetched history to that many commits from each fetched
tip and records the boundary in `shallow`; a larger depth deepens a
shallow repository.

`FETCH_HEAD` is written in the format of `git`: one line per fetched
reference, the heads that a pull merges first, the others marked
`not-for-merge`. The reflog message of each updated reference is
`fetch <remote>: fast-forward`, `fetch <remote>: forced-update` or
`fetch <remote>: storing head`.

Credentials for HTTP are taken from the address, from the variables
`NURT_GIT_TOKEN_<HOST>` and `NURT_GIT_TOKEN`, from the variables of hosting
providers or from the credential stores. Credential helpers that are programs
are not run and are reported as `credential_helper_ignored`. A password in the
address is used for the transfer and never written to the results.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `remote` | `text` | `"origin"` | Remote to fetch from. |
| `prune` | `bool` | `false` | Remove remote-tracking references that no longer exist on the remote. |
| `tags` | `bool` | `false` | Fetch all tags of the remote. |
| `depth` | `int` | none | Limit the fetched history to this many commits. |
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

Fetches the configured refspecs of `origin`.

```nurt example
git.fetch "/home/user/app"
```

```text expected
effect  git.fetch  ok  remote:origin  state: completed
```

Fetches all tags and removes the remote-tracking references of deleted
branches.

```nurt example
git.fetch "/home/user/app" remote: "upstream" prune: true tags: true
```

Deepens a shallow repository to 50 commits.

```nurt example
git.fetch "/home/user/app" depth: 50
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `repo` is not inside a repository. | Give the path of a repository. |
| `E_GIT_REF` | The remote is not configured (`unknown_remote`), a refspec selects nothing (`no_match`), or a refspec would update the checked out branch (`checked_out`). | `git.branch "<repo>" action: list` |
| `E_INVALID_VALUE` | `depth` is 0, or a refspec is invalid. | Pass `depth` of 1 or more. |
| `E_GIT_LOCKED` | A lock file of a reference exists. | Remove the stale `.lock` file after checking that no Git process runs. |
| `E_GIT_AUTH` | The remote requires credentials that are missing or were refused. | Set `NURT_GIT_TOKEN_<HOST>` and use an `https://` address. |
| `E_GIT_HOST_KEY` | The SSH host key is unknown or changed. | Pass `accept_host_key` with the fingerprint of the host. |
| `E_GIT_PROTOCOL` | The remote answered outside the protocol, or its object format differs. | Check that the address names a Git repository. |
| `E_NETWORK` | The connection failed; the problem is `dns`, `connect`, `tls`, `timeout`, `proxy` or `http_status`. | Check the address, the proxy settings and the network. |
| `E_CANCELLED` | The transfer was interrupted; no reference was changed. | Repeat the call. |
| `E_TIMEOUT` | The transfer was interrupted; no reference was changed. | Repeat the call. |

## Related

`git.pull`, `git.push`, `git.branch`, `git.clone`, `undo`.
