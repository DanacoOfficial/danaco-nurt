---
command: network.resolve
group: network
summary: Resolve DNS records of a name.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/network.resolve.md -->
<!-- Responsibility:   Documents network.resolve. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# network.resolve

## Synopsis

```text
network.resolve <name> [name: value]...
```

## Description

`network.resolve` sends a DNS query for `name` and returns the records of
the answer as `dns_record` records, one per record. The name is queried as
given, without search domains. An answer without records of the type gives
no records; a name that does not exist fails with `E_NOT_FOUND`.

Without `server`, the servers of the system configuration are asked in turn
until one answers: the `nameserver` lines of `/etc/resolv.conf` on Linux and
macOS (at most three, with `options timeout:` and `attempts`), and the DNS
servers of the network adapters on Windows. With `server`, only that server
is asked.

Each query is one UDP datagram. A truncated answer is repeated over TCP.
`type: any` sends one query per type, in the order A, AAAA, MX, TXT, NS,
CNAME, and merges the records without duplicates; it does not send a query
of type `ANY`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text` | required | Domain name. |
| `type` | `enum(a\|aaaa\|mx\|txt\|ns\|cname\|any)` | `a` | Record type. |
| `server` | `text` | none | DNS server to ask instead of the system resolver. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `dns_record` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Resolve the IPv4 addresses of a name:

```nurt example
network.resolve "example.org"
```

```text
dns_record  example.org  A  93.184.215.14  ttl 3600s
```

Ask a given server for the mail exchangers of a domain:

```nurt example
network.resolve "example.org" type: mx server: "192.0.2.53"
```

List the records of every supported type, ordered by type:

```nurt example
network.resolve "example.org" type: any |> sort by: type
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | `type` is not one of the listed values, `name` is not a valid domain name, or `server` is not an address and cannot be resolved. | Correct the value as described in the message. |
| `E_NOT_FOUND` | The server answers that the name does not exist. | Check the spelling of the name. |
| `E_NETWORK` | No server answers, or every server reports a failure (`problem` `dns`, the server in `server`). | Check the network or give another server with `server`. |
| `E_IO` | The system configuration exists but cannot be read. | Check the permissions of `/etc/resolv.conf`. |
| `E_TIMEOUT` | The program reached its time limit. | Raise the limit of the program. |
| `E_CANCELLED` | The execution was cancelled. | Run the command again. |

## Related

`network.ping`, `network.interfaces`, `http.request`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
