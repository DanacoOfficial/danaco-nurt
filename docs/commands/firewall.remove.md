---
command: firewall.remove
group: admin
summary: Remove the firewall rules that match a port, protocol, source, direction and zone.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# firewall.remove

## Synopsis

```text
firewall.remove <port> [name: value]...
```

## Description

`firewall.remove` removes the rules that match `port`, `protocol`, `source`, `direction` and `zone`. `action` limits the removal to allow or deny rules; with `any` both are removed. A call that matches no rule fails with `E_NOT_FOUND` and changes nothing. Every removed rule is one change of the target `firewall:<rule>` from `present` to `absent`; `undo` adds the rules again with `firewall.allow` or `firewall.deny`. Rules are described by `port`, `protocol`, `source`, `direction` and `zone`. `source` is the remote address of the traffic: the sender of incoming and the receiver of outgoing traffic. A port is a number from 1 to 65535 or a range `first-last`.

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
| `port` | `int \| text` | required | Port or range of ports such as 8000-8100. |
| `protocol` | `enum(tcp\|udp)` | `tcp` | Transport protocol. |
| `source` | `text` | none | Remote address or network in CIDR form: the sender of incoming and the receiver of outgoing traffic; any when omitted. |
| `direction` | `enum(inbound\|outbound)` | `inbound` | Direction of the traffic. |
| `zone` | `text` | none | Zone of firewalld or profile of Windows (domain, private, public); the default one when omitted. |
| `backend` | `enum(auto\|nftables\|firewalld\|ufw\|pf\|windows)` | `auto` | Firewall to use; auto selects the first one that is installed and running. |
| `table` | `text` | `"nurt"` | Table of the family inet that holds the rules of nftables. |
| `action` | `enum(any\|allow\|deny)` | `any` | Remove only rules with this action; both when any. |

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
| Reversibility conditions | `undo` adds the rules that were removed. |
| External use | `system_program`: `nft`, `firewall-cmd`, `ufw`, `pfctl`, `powershell`; The rules are read and changed through nft, firewall-cmd, ufw or pfctl; on Windows through the NetSecurity cmdlets of PowerShell. |

## Examples

Remove the rule for port 22.

```nurt example
firewall.remove 22 privilege: elevated
```

Remove only the deny rule for a UDP port.

```nurt example
firewall.remove 5353 protocol: udp action: deny privilege: elevated
```

Remove a rule of a named nftables table.

```nurt example
firewall.remove 8080 backend: nftables table: "nurt" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | No rule matches. | List the rules with `firewall.rules`. |
| `E_INVALID_VALUE` | `port`, `source` or `zone` is not valid. | Write a port from 1 to 65535, an address in CIDR form or a plain name. |
| `E_PRIVILEGE` | The program of the system refused the change for want of rights. | Repeat the call with `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | No program of the system that can do this was found. | Install the program or name the `backend`. |

## Related

`firewall.rules`, `firewall.allow`, `firewall.deny`.
