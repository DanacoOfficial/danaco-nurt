---
command: network.route_add
group: administration
summary: Add a route to the routing table.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# network.route_add

## Synopsis

```text
network.route_add <destination> [name: value]...
```

## Description

`network.route_add` adds a route to the routing table: `ip route add` on Linux, `route add` on macOS and `New-NetRoute` on Windows. `destination` is a network in CIDR form, a host address or `default`. A route needs a `gateway` or an `interface`. When the table already holds a route with the same destination, next hop, interface and metric, nothing is changed and the report states `unchanged`. `undo` removes the route with `network.route_remove`.

Routes and DNS are read before the change. The report holds the state before and after (`via=<gateway> dev=<interface> metric=<n>` for a route, `servers=<list>;search=<list>` for DNS), and `undo` restores it.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `destination` | `text` | required | Network in CIDR form, a host address or default. |
| `gateway` | `text` | none | Next hop. Required unless interface is given. |
| `interface` | `text` | none | Interface of the route. |
| `metric` | `int` | none | Metric of the route. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `external` |
| Reversibility | `reversible` |
| Implementation | `builtin` |
| Reversibility conditions | `undo` removes the route; a route that already exists is left alone. |
| External use | `system_program`: `ip`, `route`, `netstat`, `resolvectl`, `networksetup`, `powershell`; Routes and DNS are read and changed through ip, route, netstat, resolvectl and networksetup, and through the NetTCPIP and DnsClient cmdlets of PowerShell on Windows. |

## Examples

Route a network through a gateway.

```nurt example
network.route_add "10.9.0.0/16" gateway: "192.168.1.5" privilege: elevated
```

Route a network over an interface with a metric.

```nurt example
network.route_add "10.20.0.0/16" interface: "eth0" metric: 200 privilege: elevated
```

Set the default route.

```nurt example
network.route_add "default" gateway: "192.168.1.1" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | A value is not an address, a network or an interface name. | Correct the value. |
| `E_MISSING_PARAM` | Neither `gateway` nor `interface` is given. | Name the next hop or the interface. |
| `E_PRIVILEGE` | The program of the system refused the change for want of rights. | Repeat the call with `privilege: elevated`. |

## Related

`network.route_remove` removes routes and `network.dns_set` sets name servers.
