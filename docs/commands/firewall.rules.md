---
command: firewall.rules
group: admin
summary: List the rules of the firewall with their action, protocol, port, source, direction and zone.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# firewall.rules

## Synopsis

```text
firewall.rules [<port>] [name: value]...
```

## Description

`firewall.rules` lists the rules of the firewall as records with the fields `backend`, `place` (table and chain, or zone), `managed` (the rule was created by Nurt), `action` (`allow` or `deny`), `protocol`, `port`, `direction`, `source`, `zone` and `rule` (the rule as the firewall prints it). A rule of a shape that Nurt does not model, for example a rule without a port, has only `backend`, `place`, `managed` and `rule`. `port` and `protocol` keep only the rules that match.

Reading the rules of most firewalls needs the administrator: write `privilege: elevated`.

The firewall is chosen by `backend`. With `auto` the command takes the first that is installed and running: firewalld, then ufw, then nftables on Linux; pf on macOS; the Windows Defender Firewall on Windows.

| Backend | System | Notes |
|---|---|---|
| `nftables` | Linux | Rules live in the chains `input` and `output` of the table `table` (default `nurt`, family `inet`); the table and the chain are created when absent. The comment of a rule holds its key. `zone` is not available. |
| `firewalld` | Linux | A rule is added to the runtime configuration and to the permanent one. Plain allow rules are ports; rules with `source` and every deny rule are rich rules. `zone` defaults to the default zone of firewalld; only incoming traffic is supported. |
| `ufw` | Linux | Rules are added and deleted with `ufw`; `zone` is not available. |
| `pf` | macOS | Rules live in the anchor `nurt`, which holds only rules of Nurt and is loaded as a whole with `pfctl -a nurt -f -`. The anchor takes effect when `pf.conf` refers to it with `anchor "nurt"`. `zone` is not available. |
| `windows` | Windows | Rules are created with `New-NetFirewallRule` under the display name `nurt:<key>`; `zone` names the profile `domain`, `private` or `public`. |

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `port` | `int \| text` | none | Only rules for this port or range. |
| `protocol` | `enum(tcp\|udp)` | none | Only rules of this protocol. |
| `backend` | `enum(auto\|nftables\|firewalld\|ufw\|pf\|windows)` | `auto` | Firewall to use; auto selects the first one that is installed and running. |
| `table` | `text` | `"nurt"` | Table of the family inet that holds the rules of nftables. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `any` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |
| External use | `system_program`: `nft`, `firewall-cmd`, `ufw`, `pfctl`, `powershell`; The rules are read and changed through nft, firewall-cmd, ufw or pfctl; on Windows through the NetSecurity cmdlets of PowerShell. |

## Examples

List every rule of the firewall.

```nurt example
firewall.rules
```

List the rules of port 22 of nftables.

```nurt example
firewall.rules 22 backend: nftables
```

Show only the rules that Nurt created.

```nurt example
firewall.rules |> filter managed
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_PRIVILEGE` | The program of the system refused the change for want of rights. | Repeat the call with `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | No program of the system that can do this was found. | Install the program or name the `backend`. |
| `E_OS_ONLY` | The backend does not exist on this system. | Use `backend: auto`. |

## Related

`firewall.allow`, `firewall.deny` and `firewall.remove` change the rules.
