---
command: network.interfaces
group: network
summary: List network interfaces with addresses, MAC address, state and MTU.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# network.interfaces

## Synopsis

```text
network.interfaces
```

## Description

`network.interfaces` returns one `network_interface` record per network
interface of the host, ordered by name. Addresses of an alias such as
`eth0:1` belong to the interface `eth0`. The default gateway of an interface
is taken from the IPv4 routing table, otherwise from the IPv6 routing table.

## Parameters

The command has no parameters.

## Input and output

| Input | Output |
|---|---|
| none | `network_interface` |

| Field | Type | Description |
|---|---|---|
| `name` | `text` | Name of the interface. |
| `addresses` | `list<text>` | Addresses with prefix length, for example `192.0.2.10/24` and `2001:db8::10/64`. |
| `up` | `bool` | The interface is up. |
| `mac` | `text` | Hardware address `aa:bb:cc:dd:ee:ff`; absent for interfaces without one. |
| `mtu` | `int` | Maximum transmission unit. |
| `kind` | `enum` | `ethernet`, `wireless`, `loopback`, `virtual` or `other`. |
| `gateway` | `text` | Address of the default gateway reached through the interface. |

## Effect

Category `source`, effect `read`, reversibility `read`. The command reads the
interface tables of the operating system and changes nothing.

## Notes

| System | Addresses | State, MAC, MTU, kind | Gateway |
|---|---|---|---|
| Linux | `getifaddrs` | `/sys/class/net/<name>/{operstate,flags,address,mtu,type}`; `wireless` or `phy80211` gives `wireless`, a missing `device` gives `virtual` | `/proc/net/route`, `/proc/net/ipv6_route` |
| macOS | `getifaddrs` | link-layer entries of `getifaddrs` and their interface data; the link type and the name prefix give the kind | routing table read through `sysctl` (`NET_RT_FLAGS`) |
| Windows | `GetAdaptersAddresses` | the adapter entry; the interface type gives the kind | gateways of the adapter, IPv4 first |

On Linux, an `operstate` of `unknown`, as reported by the loopback interface
and some virtual interfaces, is resolved by the `IFF_UP` flag.

## Examples

List all interfaces:

```nurt example
network.interfaces
```

```text
network_interface  lo    127.0.0.1/8, ::1/128         up  loopback
network_interface  eth0  192.0.2.10/24, 2001:db8::10/64  up  ethernet  gateway 192.0.2.1
```

List the interfaces that are up and have a default gateway:

```nurt example
network.interfaces | where up and gateway != null
```

Show the hardware addresses of the wireless interfaces:

```nurt example
network.interfaces | where kind == "wireless" | select name, mac
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_IO` | The system refuses the list of interfaces, or `/sys/class/net` cannot be read on Linux. | Check the state of the system and run the program again. |
| `E_OS_ONLY` | The system is not Linux, macOS or Windows. | Run the program on a supported system. |

## Related

`network.ping`, `network.resolve`, `ports`.
