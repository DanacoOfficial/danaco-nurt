---
command: network.dns_set
group: admin
summary: Set the DNS servers and search domains of an interface; an empty list of servers returns to automatic servers.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# network.dns_set

## Synopsis

```text
network.dns_set <servers> [name: value]...
```

## Description

`network.dns_set` sets the name servers and, with `search`, the search domains of an interface. An empty list of `servers` returns the interface to automatic servers. `search` left out keeps the search domains.

| System | Mechanism |
|---|---|
| Linux with systemd-resolved | `resolvectl dns`, `resolvectl domain`, `resolvectl revert`; `interface` defaults to the interface of the default route |
| Linux otherwise | the file `/etc/resolv.conf`, rewritten through the journal so that `undo` restores the file; the other lines of the file are kept; a link is refused with `E_UNSUPPORTED` |
| macOS | `networksetup -setdnsservers` and `-setsearchdomains`; `interface` is the name of the network service |
| Windows | `Set-DnsClientServerAddress` for the interface alias, `Set-DnsClientGlobalSetting` for the search domains |

When the servers and domains already hold, nothing is changed. `undo` sets the previous servers and domains with `network.dns_set`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `servers` | `list<text>` | required | IP addresses of the name servers, in order; an empty list means automatic. |
| `search` | `list<text>` | none | Search domains; unchanged when omitted. |
| `interface` | `text` | none | Interface or network service; the interface of the default route when omitted on Linux. |

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
| External use | `system_program`: `ip`, `route`, `netstat`, `resolvectl`, `networksetup`, `powershell`; Routes and DNS are read and changed through ip, route, netstat, resolvectl and networksetup, and through the NetTCPIP and DnsClient cmdlets of PowerShell on Windows. |
| Reversibility conditions | `undo` sets the previous servers and domains; changes of /etc/resolv.conf are restored by the journal. |

## Examples

Use two public name servers on an interface.

```nurt example
network.dns_set ["1.1.1.1", "8.8.8.8"] interface: "eth0" privilege: elevated
```

Set servers and search domains.

```nurt example
network.dns_set ["10.0.0.2"] search: ["corp.example.com"] interface: "eth0" privilege: elevated
```

Return an interface to automatic servers.

```nurt example
network.dns_set [] interface: "eth0" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | A server is not an IP address or a domain is not a domain name. | Correct the value. |
| `E_MISSING_PARAM` | `interface` is needed and not given. | Name the interface. |
| `E_UNSUPPORTED` | `/etc/resolv.conf` is a link that another service manages. | Set the DNS in that service or enable systemd-resolved. |
| `E_PRIVILEGE` | The program of the system refused the change for want of rights. | Repeat the call with `privilege: elevated`. |

## Related

`network.route_add`, `network.route_remove`.
