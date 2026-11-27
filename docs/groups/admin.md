---
group: admin
summary: System services, the registry provider, CIM, the firewall, routes, DNS and the clock.
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Group admin

## Scope

The group lists and controls services of the operating system and queries
Windows management objects. Services have one model on all systems: the
service manager of the system (systemd, the Windows service control manager
or launchd) is hidden behind the `service` record, and the commands that
change a service take `service` records as input.

The Windows registry is not a command of this group. It is a provider of
paths: `registry:/HKCU/Software/Example` addresses a key and
`registry:/HKCU/Software/Example/Mode` a value, and the file commands of the
core group (`list`, `find`, `read`, `write`, `remove`, `copy`, `move`) work on
them. The previous value is preserved in the operation journal, so a registry
change made with `write` can be undone.

| Task | Command | Group |
|---|---|---|
| Stop a service | `stop` | core |
| Suspend or resume a process | `suspend`, `resume` | system |
| Read and change registry values | `read`, `write`, `remove` | core |
| Undo a registry change | `undo` | core |

## Commands

| Command | Category | Input and output | Reversibility | Page |
|---|---|---|---|---|
| `services` | source | none to `service` | read | [services](../commands/services.md) |
| `start` | run | `service` to `effect` | irreversible | [start](../commands/start.md) |
| `restart` | run | `service` to `effect` | irreversible | [restart](../commands/restart.md) |
| `enable` | change | `service` to `effect` | irreversible | [enable](../commands/enable.md) |
| `disable` | change | `service` to `effect` | irreversible | [disable](../commands/disable.md) |
| `windows.cim` | change | none to `cim_object`, `effect` | read | [windows.cim](../commands/windows.cim.md) |
| `firewall.rules` | source | none to `map` | read | [firewall.rules](../commands/firewall.rules.md) |
| `firewall.allow` | change | none to `effect` | reversible | [firewall.allow](../commands/firewall.allow.md) |
| `firewall.deny` | change | none to `effect` | reversible | [firewall.deny](../commands/firewall.deny.md) |
| `firewall.remove` | change | none to `effect` | reversible | [firewall.remove](../commands/firewall.remove.md) |
| `network.route_add` | change | none to `effect` | reversible | [network.route_add](../commands/network.route_add.md) |
| `network.route_remove` | change | none to `effect` | reversible | [network.route_remove](../commands/network.route_remove.md) |
| `network.dns_set` | change | none to `effect` | reversible | [network.dns_set](../commands/network.dns_set.md) |
| `clock.timezone_set` | change | none to `effect` | reversible | [clock.timezone_set](../commands/clock.timezone_set.md) |
| `clock.set` | change | none to `effect` | irreversible | [clock.set](../commands/clock.set.md) |
| `clock.ntp` | change | none to `effect` | reversible | [clock.ntp](../commands/clock.ntp.md) |

## Records

| Record | Required fields |
|---|---|
| `service` | `name`, `state`, `start_type`, `manager` |
| `cim_object` | `class`, `path`, `properties` |
| `effect` | `status`, `kind` |

The values of `service.state` are `running`, `stopped`, `starting`,
`stopping`, `failed`, `paused` and `unknown`. The values of
`service.start_type` are `automatic`, `manual`, `disabled`, `on_demand`,
`static` and `unknown`. The field `manager` is `systemd`, `windows` or
`launchd`. The field `state_raw` keeps the state as the service manager
reports it.

The `effect` record of `start`, `restart`, `enable` and `disable` names the
service in `target` (for example `service:nginx`), the action in `action`, and
the state of the service in `before` and `after`.

## Service model

`services` selects services by `name` (text or regular expression), by
`state` and by `scope`. `scope` selects the instance of the service manager:

| Scope | Service manager instance |
|---|---|
| `system` | services of the computer; default |
| `user` | services of the account that runs the program |

`scope` of `services` is not the `scope` of `environment`; the values and the
meaning differ, see the group `system`.

`start` and `restart` take the services of the input and wait up to `wait`
(default `10s`) for the requested state. `enable` makes the services start at
boot, or at login for the scope `user`. `disable` removes that start. A
program has one stage that changes the world, so `start`, `restart`, `enable`
and `disable` are the last stage that changes anything: `services name: "nginx"
|> restart` selects in the read step and restarts in the apply step.

Changing the state of a service is an effect outside the file system. The
operation journal records it as irreversible, `undo` does not reverse it, and
the effect report lists it among the irreversible effects. A preview shows the
services that would be changed and changes nothing.

## Firewall, routes, DNS and the clock

`firewall.rules`, `firewall.allow`, `firewall.deny` and `firewall.remove` read
and change the rules of the firewall of the system: nftables, firewalld or ufw
on Linux, pf on macOS and the Windows Defender Firewall. `network.route_add`,
`network.route_remove` and `network.dns_set` change the routing table and the
name servers. `clock.timezone_set`, `clock.ntp` and `clock.set` change the time
zone, network time and the clock.

Every one of these commands changes the system and accepts `privilege`; they
need the administrator, so a call is written with `privilege: elevated`. The
stage never starts `sudo` itself: the executor runs the stage with the
elevated process, and a program that reports missing rights ends the call with
`E_PRIVILEGE`.

The report of a change holds a change of the action `state` for one target
with the value before and after: `firewall:<rule>` (`absent` or `present`),
`route:<destination>`, `dns:<interface>`, `clock:timezone` and `clock:ntp`.
From these values the engine derives the program that `undo` runs, with the
rights of the original call:

| Command | `undo` runs |
|---|---|
| `firewall.allow`, `firewall.deny` | `firewall.remove` of the rule |
| `firewall.remove` | `firewall.allow` or `firewall.deny` of every removed rule |
| `network.route_add` | `network.route_remove` of the route |
| `network.route_remove` | `network.route_add` of every removed route |
| `network.dns_set` | `network.dns_set` with the previous servers and domains; a change of `/etc/resolv.conf` is restored by the journal |
| `clock.timezone_set` | `clock.timezone_set` with the previous zone |
| `clock.ntp` | `clock.ntp` with the previous state |
| `clock.set` | irreversible |

A change that the system already holds (a rule that exists, the same zone) is
reported as `unchanged`, opens no journal entry and runs no program. A preview
reads the system and lists the effects without changing anything.

## Platform differences

| Aspect | Linux | macOS | Windows |
|---|---|---|---|
| Service manager | systemd, through its D-Bus interface | launchd | service control manager |
| `manager` value | `systemd` | `launchd` | `windows` |
| Reading services | D-Bus | property list files and `launchctl` | service control manager API |
| Changing services | D-Bus | `launchctl` | service control manager API |
| `scope: user` | systemd user instance of the account | launchd user jobs of the account | services of the account, when the system provides them |
| `windows.cim` | `E_OS_ONLY` | `E_OS_ONLY` | WMI over COM |

Changing a service of the scope `system` requires a privilege. Without it the
command fails with `E_PRIVILEGE`, and `fix` names the privilege. An error of
the service manager is reported as `E_SERVICE`.

`windows.cim` queries instances of a CIM class in `namespace` (default
`root/cimv2`) with an optional WQL `where` condition and a list of
`properties`. On Linux and macOS it fails with `E_OS_ONLY`.

## External programs

| Command | Marker | Condition |
|---|---|---|
| `services` | [X] | macOS only: the state of services is read through `launchctl` |
| `start` | [X] | macOS only: services are started through `launchctl` |
| `restart` | [X] | macOS only: services are restarted through `launchctl` |
| `enable` | [X] | macOS only: services are enabled through `launchctl` |
| `disable` | [X] | macOS only: services are disabled through `launchctl` |
| `firewall.rules` | [X] | `nft`, `firewall-cmd`, `ufw` or `pfctl` read the rules; Windows uses PowerShell |
| `firewall.allow` | [X] | the same programs change the rules |
| `firewall.deny` | [X] | the same programs change the rules |
| `firewall.remove` | [X] | the same programs change the rules |
| `network.route_add` | [X] | `ip`, `route` or PowerShell |
| `network.route_remove` | [X] | `ip`, `route` or PowerShell |
| `network.dns_set` | [X] | `resolvectl`, `networksetup` or PowerShell; `/etc/resolv.conf` is written without a program |
| `clock.timezone_set` | [X] | `timedatectl`, `systemsetup` or `tzutil` |
| `clock.set` | [X] | `date` or PowerShell |
| `clock.ntp` | [X] | `timedatectl`, `systemsetup` or PowerShell |

The [X] marker names a component of the operating system that a command
starts as a program. macOS offers no public interface for these operations, so `launchctl` is the
one component of the operating system that the commands of this group start. They start it through the process interface and report it in the check of
the program. Linux and Windows use system interfaces and start no program. When `launchctl` is missing on macOS, the
command fails with `E_EXTERNAL_MISSING`.

## Examples

Restart the service `nginx`:

```nurt example
services name: "nginx" |> restart
```

List the running services that start automatically:

```nurt example
services state: "running" |> filter start_type == "automatic" |> select name state
```

Restart a service and wait up to 30 seconds for it to run:

```nurt example
services "nginx" |> restart wait: 30s
```

Stop a service from starting at boot:

```nurt example
services name: "cups" |> disable
```

Read a registry value and then change it; the previous value stays in the
journal:

```nurt example
read "registry:/HKCU/Software/Example/Mode"
write "registry:/HKCU/Software/Example/Mode" "3" value_type: dword mode: overwrite
```

Read two properties of the operating system on Windows:

```nurt example
windows.cim "Win32_OperatingSystem" properties: ["Caption", "Version"]
```

## Related

[Group system](system.md) lists processes and ports. [Group schedule](schedule.md)
runs programs at set times. `stop` of the core group stops services.
