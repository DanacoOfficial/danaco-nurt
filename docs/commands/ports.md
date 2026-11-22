---
command: ports
group: system
summary: List network ports and the processes that hold them.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# ports

## Synopsis

```text
ports [name: value]...
```

## Description

`ports` lists the TCP and UDP sockets of the host as `port` records, ordered
by protocol, local port, local address and process. With `state: listen` the
output holds listening TCP sockets and UDP sockets without a remote address;
with `state: all` it holds every socket, including established and closing
connections.

`local` is the local address and port as `address:port`, with an IPv6 address in
square brackets, for example `[::1]:8080`. `remote`, `remote_address` and
`remote_port` are set only for sockets with a peer. UDP sockets have the
state `none`. `pid` and `process` identify the process that holds the socket.

| System | Socket tables | Holding process |
|---|---|---|
| Linux | `/proc/net/tcp`, `tcp6`, `udp`, `udp6` | descriptor in `/proc/<pid>/fd` that links to the socket inode |
| macOS | `proc_pidfdinfo(PROC_PIDFDSOCKETINFO)` for each process | the process listed |
| Windows | `GetExtendedTcpTable`, `GetExtendedUdpTable` | process identifier of the table row |

On Linux the holding process of a socket of another account is known only to
a privileged account; such sockets keep an empty `pid`. On macOS the sockets
of processes of other accounts are listed only for a privileged account. In
both cases the report carries one `source_unavailable` note with the number
of affected sockets or processes. TCP states map to `listen`, `established`,
`syn_sent`, `syn_received`, `fin_wait`, `close_wait`, `closing`, `last_ack`,
`time_wait` and `closed`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `state` | `enum(listen\|all)` | `listen` | Listening ports only, or all sockets. |
| `protocol` | `enum(tcp\|udp\|any)` | `any` | Transport protocol. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `port` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

List the listening TCP ports:

```nurt example
ports state: listen protocol: tcp
```

List all UDP sockets:

```nurt example
ports state: all protocol: udp
```

List all sockets with their states and processes:

```nurt example
ports state: all
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | `state` or `protocol` has a value outside the enumeration. | Use one of the listed values. |
| `E_PRIVILEGE` | The socket tables are not readable by the account. | Run with the privilege named in `required`. |
| `E_IO` | A socket table cannot be read. | Check the state of the system and run the program again. |

## Related

`processes`, `network.interfaces`, `open_files`.
