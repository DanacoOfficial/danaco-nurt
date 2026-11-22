---
command: http.download
group: network
summary: Download a file over HTTP, resuming a partial download when possible.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# http.download

## Synopsis

```text
http.download <url> <to> [name: value]...
```

## Description

`http.download` sends a `GET` request to `url` and writes the body of the
response to the file `to`. The body is stored as received: the request asks
for `Accept-Encoding: identity`, and redirects are followed.

The body is first written to the partial file `<to>.part` in the directory
of the target. The file `<to>.part.meta` records the address and the
validator of the response: a strong `ETag`, otherwise `Last-Modified`. When a
download ends early (connection closed, time limit, cancellation), both files
stay. The next run for the same address and target with `resume: true` sends
`Range: bytes=<size of the partial file>-` with `If-Range: <validator>`:

| Answer of the server | Result |
|---|---|
| `206` from the size of the partial file | The rest is appended; `resumed_from` is the size of the partial file. |
| `200` | The resource changed or the server ignores ranges; the download starts again and `resumed_from` is 0. |
| `416` with the size of the partial file as the complete length | The partial file already holds the whole content; no bytes are transferred. |

With `hash`, the digest of the whole content is computed while the bytes
arrive, including the bytes of a resumed partial file. A mismatch removes the
partial file and leaves the target unchanged.

After a complete transfer with a matching digest, the content replaces the
target in one step and the partial file is removed. The directory of `to`
must exist.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `url` | `text` | required | Address of the file; `https://` is assumed when the scheme is missing. Position 1. |
| `to` | `path` | required | File to write. Position 2. |
| `resume` | `bool` | `true` | Continue a partial download of the same address; `false` starts from the first byte. |
| `hash` | `text` | none | Expected digest `<algorithm>:<hex>` with the algorithm `sha256`, `sha512` or `blake3`; case is ignored. |
| `headers` | `map<text>` | none | Request headers. `Range` and `If-Range` are set by the command and cannot be given. Values are secret and do not appear in reports. |
| `timeout` | `duration` | `300s` | Time limit of the download, including name resolution, connection, redirects and the transfer of the body. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

The `effect` record has `kind` `http.download`, one entry in `changes` for
the target (`action` `create` or `modify`, `size_before`, `size_after`,
`hash_after` as `<algorithm>:<hex>`) and the download object in `after`:

Progress events of phase `receive` report the bytes received.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |
| Reversibility conditions | The written file is reversible; the request itself only reads from the network. |

The request only reads from the network and is not recorded as an
irreversible effect. The partial file and its description are written outside
the journal and are not undone; they are removed after a successful download
and after a digest mismatch.

## Examples

Download an archive and check its SHA-256 digest:

```nurt example
http.download "https://example.org/dist/tool-1.4.2.tar.gz" "/home/user/tool-1.4.2.tar.gz" hash: "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
```

```text expected
effect  http.download  ok  /home/user/tool-1.4.2.tar.gz  create
```

Download an image again from the first byte, with a longer time limit:

```nurt example
http.download "https://example.org/images/disk.img" "/srv/images/disk.img" resume: false timeout: 30m
```

Download a build artifact with an additional request header:

```nurt example
http.download "https://artifacts.example.org/builds/42/app.zip" "/home/user/app.zip" headers: {Accept: "application/zip"}
```

Show the download object, which tells whether an earlier partial download
was continued:

```nurt example
http.download "https://example.org/data.csv" "/home/user/data.csv" |> select after
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_ARG` | `url` is not a valid HTTP or HTTPS address. | Give an address such as `https://example.org/file`. |
| `E_INVALID_VALUE` | `hash` has no known algorithm or a wrong number of digits, `headers` sets `Range` or `If-Range`, `timeout` is zero, or `to` is not a regular file. | Correct the value as described in the message. |
| `E_NOT_FOUND` | The directory of `to` does not exist. | Create the directory, then run the command again. |
| `E_NETWORK` | The server cannot be reached, the TLS handshake fails, the connection closes before the end of the body (`problem` `incomplete`), or the server answers with a status of 400 or above (`problem` `http_status`, the status in `status`). | Check the address and the network; run the command again to resume an incomplete download. |
| `E_HASH_MISMATCH` | The digest of the content differs from `hash`. | Check the expected digest and the address. |
| `E_TIMEOUT` | The download did not finish within `timeout`. | Increase `timeout`; the next run resumes the partial download. |
| `E_CANCELLED` | The execution was cancelled. | Run the command again to resume. |
| `E_PERMISSION`, `E_NO_SPACE`, `E_IO` | The partial file or the target cannot be written. | Check the permissions and the free space of the volume. |
| `E_STALE` | The target was created by another process during the download. | Run the command again. |

## Related

`http.request`, `hash`, `undo`.
