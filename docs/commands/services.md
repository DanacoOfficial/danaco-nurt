---
command: services
group: admin
summary: List system or user services with their state and start type.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# services

## Synopsis

```text
services [<name>] [state: <text>] [scope: system|user]
```

## Description

`services` lists the services of the service manager of the system: systemd
on Linux, the service control manager on Windows and launchd on macOS. It
starts a pipeline and produces one `service` record per service; on Linux the
records are sorted by name.

A text `name` selects one service the way the manager resolves names: on
Linux `nginx` and `nginx.service` name the same unit and template instances
such as `getty@tty1` are accepted; on Windows the comparison ignores case.
When the manager knows no such service, the command fails with `E_NOT_FOUND`.
A regular expression `name` keeps the services whose name matches it.

`state` keeps the services whose state, raw state, or one part of the raw
state separated by `/` equals the text, ignoring case. `state: "failed"`
matches the common state; `state: "exited"` matches the systemd raw state
`active/exited`.

On Linux the command merges the loaded units with the unit files that are
installed but not loaded, so a disabled service that never ran is listed as
`stopped`. Units of other types and unit templates are not listed. The manager
is reached over D-Bus; no external program is run. `scope: user` reads the
manager of the current account on the session bus.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text \| regex` | none, positional 1 | Service name, or a pattern the name matches. |
| `state` | `text` | none | Only services in this state, raw state or part of the raw state. |
| `scope` | `enum(system\|user)` | `system` | System services or services of the current account. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `services` | none | `service` |

The `service` record holds `name`, `state` (`running`, `stopped`, `starting`,
`stopping`, `failed`, `paused`, `unknown`), `start_type` (`automatic`,
`manual`, `disabled`, `on_demand`, `static`, `unknown`), `manager`, `scope`,
and where the manager reports them `state_raw`, `description`, `pid`, `path`,
`account` and `os_raw`.

| systemd `ActiveState` | `state` |
|---|---|
| `active` | `running`; `stopped` for `active/exited`, a unit that ran to completion |
| `reloading` | `running` |
| `inactive` | `stopped` |
| `activating` | `starting` |
| `deactivating` | `stopping` |
| `failed` | `failed` |
| any other value | `unknown` |

| systemd `UnitFileState` | `start_type` |
|---|---|
| `enabled`, `enabled-runtime` | `automatic` |
| `disabled`, `masked`, `masked-runtime` | `disabled` |
| `static` | `static` |
| `indirect` | `on_demand` |
| no unit file | `manual` |
| `linked`, `generated`, `transient`, `alias`, `bad` | `unknown` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read`: nothing is changed, so there is nothing to undo |
| External programs | `launchctl` on macOS only |
| Waits | no |

## Examples

Show one service:

```nurt example
services name: "cron"
```

List the failed services:

```nurt example
services state: "failed"
```

List the running services of the current account:

```nurt example
services scope: user state: "running"
```

Restart one service:

```nurt example
services name: "nginx" |> restart
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | No service has the given name. | List the services with `services` and use a listed name. |
| `E_REGEX` | The pattern of `name` is not a valid regular expression. | Correct the pattern. |
| `E_INVALID_VALUE` | `scope` is neither `system` nor `user`. | Use `scope: system` or `scope: user`. |
| `E_PRIVILEGE` | The manager refused to show the service; `required` names the privilege. | Run as an account with that privilege. |
| `E_SERVICE` | The manager reported an error or cannot be reached. `os_code` is `no_systemd` when systemd is not the service manager and `no_user_bus` when the account has no session bus. | Use `scope: system`, or run on a system managed by systemd. |
| `E_UNSUPPORTED` | The scope is not available on this system. | Use `scope: system`. |

## Related

`start`, `restart`, `enable` and `disable` act on the records of `services`;
`stop` stops them. `logs unit:` shows the log of a service. The `admin` group
page lists the other commands of the group.
