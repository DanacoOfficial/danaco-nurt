---
command: disable
group: admin
summary: Stop the input services from starting automatically.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# disable

## Synopsis

```text
services … |> disable
```

## Description

`disable` stops each input service from starting automatically. On Linux the
unit file is disabled and the manager reloads its configuration; a masked unit
already counts as `disabled`. `before` and `after` hold the start type. A
running service keeps running; `stop` stops it.

Each input service is read before the action. A service whose start type is
already `disabled` is not passed to the manager; its effect record has
`before` equal to `after` and the notable code `unchanged`. For every other
service the operation is marked irreversible with `service_changed` before
the manager is asked to act. Services are processed in input order; an error
stops the processing of the remaining services.

## Parameters

The command has no parameters.

## Input and output

| Form | Input | Output |
|---|---|---|
| `services … \|> disable` | `service` | `effect` |

Each `effect` record holds `kind: "disable"`, `target` (`service:<name>`),
`action`, `before` and `after`. A preview shows the same records with
`status: planned` and the expected `after`.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `external` |
| Reversibility | `irreversible`: the journal keeps no previous start type; `enable` reverts the change by hand; the report lists `service_changed` |
| External programs | `launchctl` on macOS only |
| Waits | no |

## Examples

Disable one service:

```nurt example
services name: "cups" |> disable
```

Disable a service of the current account:

```nurt example
services name: "syncthing" scope: user |> disable
```

Disable the stopped services whose name starts with `legacy`:

```nurt example
services name: /^legacy/ state: "stopped" |> disable
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | The service no longer exists. | Read the services again with `services`. |
| `E_PRIVILEGE` | The manager refused the action; `required` names the privilege, `root` on Linux and `administrator` on Windows. Interactive authorization is never requested. | Run as an account with that privilege. |
| `E_SERVICE` | The manager reported an error; `os_code` and `os_message` hold the error of the manager. | Correct the cause named in `os_message`. |
| `E_UNSUPPORTED` | The record comes from the manager of another system. | Read the services on this system with `services`. |
| `E_CANCELLED` | The execution was cancelled. | None; the cancellation was requested by the caller. |

## Related

`services` produces the input records. `enable` makes a service start
automatically and `stop` stops it now. The `admin` group page lists the other
commands of the group.
