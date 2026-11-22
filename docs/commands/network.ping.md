---
command: network.ping
group: network
summary: Send ICMP echo requests to a host and report round-trip times.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# network.ping

## Synopsis

```text
network.ping <host> [count: <int>] [timeout: <duration>]
```

## Description

`network.ping` sends `count` ICMP echo requests to `host`, one second apart,
and returns one `ping_result` record with the number of replies and the
round-trip times. A host name is resolved by the resolver of the operating
system, which also reads the hosts file; the first address is used. An
address is used as given, IPv4 or IPv6.

A request without a reply within `timeout` counts as lost. Replies are
matched to their requests by sequence number and by a random token in the
payload, so replies to other programs are ignored.

| System | Mechanism |
|---|---|
| Linux | ICMP datagram socket (`SOCK_DGRAM` with `IPPROTO_ICMP` or `IPPROTO_ICMPV6`) |
| macOS | ICMP datagram socket |
| Windows | `IcmpSendEcho` for IPv4, `Icmp6SendEcho2` for IPv6 |

Raw sockets are not used.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `host` | `text` | required | Host name or address. Position 1. |
| `count` | `int` | `4` | Number of requests; at least 1. |
| `timeout` | `duration` | `2s` | Time to wait for each reply; longer than zero. |

## Input and output

| Input | Output |
|---|---|
| none | `ping_result` |

| Field | Type | Description |
|---|---|---|
| `host` | `text` | Host as given. |
| `address` | `text` | Address the requests were sent to. |
| `sent` | `int` | Requests sent. |
| `received` | `int` | Replies received. |
| `loss` | `number` | Lost requests in percent. |
| `rtt_min`, `rtt_avg`, `rtt_max` | `number` | Shortest, average and longest round trip in milliseconds; absent without replies. |
| `rtt_stddev` | `number` | Population standard deviation of the round trips in milliseconds. |

Progress events of phase `echo` report the requests sent.

## Effect

Category `source`, effect `read`, reversibility `read`. The command sends
echo requests and changes nothing on the host.

On Linux a group of the account must be in `net.ipv4.ping_group_range`; on
macOS and Windows no privilege is required. When Linux refuses the socket,
the error names the current value of `net.ipv4.ping_group_range` and offers
a fix that allows every group.

## Examples

Send four requests to a host:

```nurt example
network.ping "example.org"
```

```text
ping_result  example.org  93.184.215.14  sent 4  received 4  loss 0
```

Send ten requests to an IPv6 address and wait up to one second for each:

```nurt example
network.ping "2001:db8::1" count: 10 timeout: 1s
```

Report only the loss and the average round trip of a gateway:

```nurt example
network.ping "192.168.1.1" count: 20 | select loss, rtt_avg
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | `count` is below 1, `timeout` is zero, or `host` is empty. | Give a positive count and time limit and a host. |
| `E_NETWORK` | The host name cannot be resolved (`problem` `dns`) or a request cannot be sent (`problem` `icmp`). | Check the name and the network. |
| `E_PRIVILEGE` | The system refuses an ICMP socket to the account. | Run the program in `fix`, which widens `net.ipv4.ping_group_range`. |
| `E_TIMEOUT` | The program reached its time limit. | Lower `count` or raise the limit of the program. |
| `E_CANCELLED` | The execution was cancelled. | Run the command again. |

## Related

`network.resolve`, `network.interfaces`, `http.request`.
