---
command: network.route_remove
group: administration
summary: Remove the routes that match a destination, next hop, interface and metric.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/network.route_remove.md -->
<!-- Responsibility:   Documents network.route_remove. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# network.route_remove

## Synopsis

```text
network.route_remove <destination> [name: value]...
```

## Description

`network.route_remove` removes the routes that match `destination` and, when given, `gateway`, `interface` and `metric`: `ip route del` on Linux, `route delete` on macOS and `Remove-NetRoute` on Windows. A call that matches no route fails with `E_NOT_FOUND`. Every removed route is recorded with its next hop, interface and metric, and `undo` adds it again with `network.route_add`.

Routes and DNS are read before the change. The report holds the state before and after (`via=<gateway> dev=<interface> metric=<n>` for a route, `servers=<list>;search=<list>` for DNS), and `undo` restores it.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `destination` | `text` | required | Network in CIDR form, a host address or default. |
| `gateway` | `text` | none | Next hop. Only routes through this next hop. |
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
| Reversibility conditions | `undo` adds the routes that were removed. |
| External use | `system_program`: `ip`, `route`, `netstat`, `resolvectl`, `networksetup`, `powershell`; Routes and DNS are read and changed through ip, route, netstat, resolvectl and networksetup, and through the NetTCPIP and DnsClient cmdlets of PowerShell on Windows. |

## Examples

Remove a route to a network.

```nurt example
network.route_remove "10.9.0.0/16" privilege: elevated
```

Remove the route that uses one next hop.

```nurt example
network.route_remove "10.9.0.0/16" gateway: "203.0.113.5" privilege: elevated
```

Remove the route of one interface.

```nurt example
network.route_remove "10.20.0.0/16" interface: "eth0" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | No route matches. | Read the routes of the system. |
| `E_INVALID_VALUE` | A value is not an address, a network or an interface name. | Correct the value. |
| `E_PRIVILEGE` | The program of the system refused the change for want of rights. | Repeat the call with `privilege: elevated`. |

## Related

`network.route_add`, `network.dns_set`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
