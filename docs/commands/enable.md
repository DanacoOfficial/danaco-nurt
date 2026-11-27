---
command: enable
group: admin
summary: Make the input services start automatically at boot or login.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# enable

## Synopsis

```text
service |> enable
```

## Description

`enable` makes each input service start automatically: at boot for system
services and at login for services of the current account. On Linux the unit
file is enabled and the manager reloads its configuration. `before` and
`after` hold the start type; a unit without an installation section keeps
`static`. The service is not started; `start` starts it.

Each input service is read before the action. A service whose start type is
already `automatic` is not passed to the manager; its effect record has
`before` equal to `after` and the notable code `unchanged`. For every other
service the operation is marked irreversible with `service_changed` before
the manager is asked to act. Services are processed in input order; an error
stops the processing of the remaining services.

**Privilege.** System units need the administrator: write `privilege: elevated` on the stage. `undo` of the entry disables the services again, when their start type before was `disabled`.

```nurt
services name: "cron" |> enable privilege: elevated
```

## Parameters

The command has no parameters.

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `service` | `effect` |

Each `effect` record holds `kind: "enable"`, `target` (`service:<name>`),
`action`, `before` and `after`. A preview shows the same records with
`status: planned` and the expected `after`.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `external` |
| Reversibility | `irreversible` |
| Implementation | `builtin` |
| External use | `system_program`: `launchctl`; On macOS services are enabled through the launchctl component of the operating system. |

## Examples

Enable one service:

```nurt example
services name: "cron" |> enable
```

Enable a service of the current account:

```nurt example
services name: "syncthing" scope: user |> enable
```

Enable the services whose name starts with `backup`:

```nurt example
services name: /^backup/ |> enable
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | The service no longer exists. | Read the services again with `services`. |
| `E_PRIVILEGE` | The manager refused the action; `required` names the privilege, `root` on Linux and `administrator` on Windows. Interactive authorization is never requested. | Run as an account with that privilege. |
| `E_SERVICE` | The manager reported an error, for example a masked unit; `os_code` and `os_message` hold the error of the manager. | Correct the cause named in `os_message`. |
| `E_UNSUPPORTED` | The record comes from the manager of another system. | Read the services on this system with `services`. |
| `E_CANCELLED` | The execution was cancelled. | None; the cancellation was requested by the caller. |

## Related

`services` produces the input records. `disable` stops a service from starting
automatically and `start` starts it now. The `admin` group page lists the
other commands of the group.
