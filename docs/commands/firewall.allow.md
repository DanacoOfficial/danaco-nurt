---
command: firewall.allow
group: admin
summary: Allow traffic on a port.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# firewall.allow

## Synopsis

```text
firewall.allow <port> [name: value]...
```

## Description

`firewall.allow` adds a rule that lets traffic through on a port. A rule that is already in the firewall is not added again: the report states `unchanged` and nothing is marked. The call is a change of the target `firewall:<rule>` from `absent` to `present`; `undo` removes the rule again with `firewall.remove`, in the same backend and table. Rules are described by `port`, `protocol`, `source`, `direction` and `zone`. `source` is the remote address of the traffic: the sender of incoming and the receiver of outgoing traffic. A port is a number from 1 to 65535 or a range `first-last`.

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
| Reversibility conditions | `undo` removes the rule that was added; a rule that already exists is left alone. |
| External use | `system_program`: `nft`, `firewall-cmd`, `ufw`, `pfctl`, `powershell`; The rules are read and changed through nft, firewall-cmd, ufw or pfctl; on Windows through the NetSecurity cmdlets of PowerShell. |

## Examples

Allow port 22 for TCP.

```nurt example
firewall.allow 22 privilege: elevated
```

Allow a range of UDP ports from one network.

```nurt example
firewall.allow "8000-8100" protocol: udp source: "10.0.0.0/8" privilege: elevated
```

Use nftables and a named table, outgoing traffic.

```nurt example
firewall.allow 443 direction: outbound backend: nftables table: "nurt" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | `port`, `source` or `zone` is not valid. | Write a port from 1 to 65535, an address in CIDR form or a plain name. |
| `E_UNSUPPORTED` | The backend cannot express the field, for example `zone` on ufw or `direction: outbound` on firewalld. | Leave the field out or choose another `backend`. |
| `E_PRIVILEGE` | The program of the system refused the change for want of rights. | Repeat the call with `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | No program of the system that can do this was found. | Install the program or name the `backend`. |

## Related

`firewall.rules` lists the rules, `firewall.remove` removes them, and `firewall.allow` is the opposite of `firewall.deny`.
