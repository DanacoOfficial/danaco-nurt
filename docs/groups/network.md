---
group: network
summary: HTTP requests, downloads, name resolution, ping and network interfaces.
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Group network

## Scope

The group sends HTTP requests, downloads files, resolves DNS names, measures
round-trip times and lists the network interfaces of the host. All commands
are part of Danaco Nurt; none of them starts a program.

| Task | Command | Group |
|---|---|---|
| Read or write a local file | `read`, `write` | core |
| Extract a downloaded archive | `archive.extract` | archive |
| Undo a download | `undo` | core |
| Check which hosts a program contacts | `nurt_check` | all groups |

The check of a program (`nurt_check`) lists the network targets of
`http.request` and `http.download` before the program runs, so an embedding
application can apply its own rules.

## Commands

| Command | Category | Input and output | Reversibility | Page |
|---|---|---|---|---|
| `http.request` | change | none to `response` | read, by method | [http.request](../commands/http.request.md) |
| `http.download` | change | none to `effect` | reversible | [http.download](../commands/http.download.md) |
| `network.ping` | source | none to `ping_result` | read | [network.ping](../commands/network.ping.md) |
| `network.resolve` | source | none to `dns_record` | read | [network.resolve](../commands/network.resolve.md) |
| `network.interfaces` | source | none to `network_interface` | read | [network.interfaces](../commands/network.interfaces.md) |

## Records

| Record | Required fields |
|---|---|
| `response` | `url`, `status`, `headers`, `body`, `bytes`, `elapsed`, `final_url` |
| `ping_result` | `host`, `address`, `sent`, `received`, `loss` |
| `dns_record` | `name`, `type`, `value`, `ttl` |
| `network_interface` | `name`, `addresses`, `up` |
| `effect` | `status`, `kind` |

Round-trip times of `ping_result` (`rtt_min`, `rtt_avg`, `rtt_max`,
`rtt_stddev`) are in milliseconds. `loss` is a percentage.

## HTTP requests

The status of the HTTP response is data. A response with the status 404 or
503 is a successful result of `http.request`; the status is the field `status`
of the record, and a program checks it with `filter` or `assert`. Only a
failure to obtain a response is an error.

The parameter `as` selects the form of the body: `raw` (default), `text`, or
`markdown` for a page converted from HTML. The parameter `extract` selects
what is returned from a page: `content`, `headers`, `links`, `headings`,
`meta` or `tables`. With several addresses in `url`, the command produces one
record per address.

The reversibility of `http.request` depends on the method:

| Method | Effect | Reversibility |
|---|---|---|
| `get`, `head`, `options` | read | read |
| `post`, `put`, `patch`, `delete` | external | irreversible |
| any, with `save` | file written | reversible, restored by `undo` |

An irreversible request is listed among the irreversible effects of the
effect report.

`http.download` writes the response body to the path in `to` and resumes a
partial download when the server supports it (`resume: true`). With `hash`,
the downloaded file is verified against the given digest, for example
`sha256:` followed by the digest. A file with a different digest is not kept,
and the command fails with `E_HASH_MISMATCH`. The file write goes through the
operation journal, so `undo` removes the downloaded file or restores the file
that it replaced.

## Platform differences

| Command | Linux | macOS | Windows |
|---|---|---|---|
| `http.request`, `http.download` | identical behavior | identical behavior | identical behavior |
| `network.ping` | ICMP echo through the system | ICMP echo through the system | ICMP echo through the system API |
| `network.resolve` | resolver of the system, or the host in `server` | same | same |
| `network.interfaces` | interfaces of the system | interfaces of the system | adapters of the system |

The commands report these errors:

| Code | Meaning |
|---|---|
| `E_NETWORK` | Name resolution, connection, TLS, proxy, redirect or protocol failure. |
| `E_HASH_MISMATCH` | The digest of a download differs from `hash`. |
| `E_TIMEOUT` | The `timeout` of the command or of the program elapsed. |
| `E_CANCELLED` | The execution was cancelled. |

The default `timeout` of `http.download` is `300s`, longer than the default
time limit of a program. When the program does not set `timeout`, its limit is
raised to the longest explicit limit of its commands.

## External programs

No command of the group starts a program. The group has no [X], [P] or [U]
marker.

## Examples

Send a request and check that exactly one response arrived:

```nurt example
http.request "https://api.example.com/health" |> assert count: 1
```

Check that the status is a success. A status of 400 or above fails the
`assert` and not the request:

```nurt example
http.request "https://example.com/health" |> assert all: status < 400
```

Read a page as Markdown:

```nurt example
http.request "https://example.com/docs" as: markdown
```

Send JSON with the method `post`:

```nurt example
http.request "https://example.com/api/items" method: post json: {name: "demo"}
```

Download a file and verify its digest:

```nurt example
http.download "https://example.com/a.tar.gz" "/home/user/dl/a.tar.gz" hash: "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
```

Measure three round trips to a host:

```nurt example
network.ping "example.com" count: 3
```

Resolve the mail exchangers of a domain:

```nurt example
network.resolve "example.com" type: mx
```

List the interfaces that are up:

```nurt example
network.interfaces |> filter up |> select name addresses
```

## Related

[Group schedule](schedule.md) runs requests and downloads at set times.
[Group system](system.md) lists ports and the processes that hold them.
