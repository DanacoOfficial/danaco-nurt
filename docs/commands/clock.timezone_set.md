---
command: clock.timezone_set
group: admin
summary: Set the time zone of the system.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# clock.timezone_set

## Synopsis

```text
clock.timezone_set <zone>
```

## Description

`clock.timezone_set` sets the time zone of the system: `timedatectl set-timezone` on Linux, `systemsetup -settimezone` on macOS and `tzutil /s` on Windows, where `zone` is a Windows identifier such as `W. Europe Standard Time`. A zone that the system already has changes nothing. `undo` sets the previous zone.

The previous value is read before the change and recorded as the target `clock:timezone`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `zone` | `text` | required | Name of the time zone, for example Europe/Warsaw; on Windows the identifier such as W. Europe Standard Time. |

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
| External use | `system_program`: `timedatectl`, `systemsetup`, `tzutil`, `date`, `powershell`; The clock is read and set through timedatectl, systemsetup, tzutil, date and, on Windows, PowerShell for the time service. |
| Reversibility conditions | `undo` sets the previous time zone. |

## Examples

Set the time zone of the system.

```nurt example
clock.timezone_set "Europe/Warsaw" privilege: elevated
```

Use UTC.

```nurt example
clock.timezone_set "UTC" privilege: elevated
```

Set a Windows time zone.

```nurt example
clock.timezone_set "W. Europe Standard Time" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | `zone` is not a time zone name. | Write a name of the time zone database. |
| `E_PRIVILEGE` | The program of the system refused the change for want of rights. | Repeat the call with `privilege: elevated`. |
| `E_STAGE_FAILED` | The system does not know the zone. | List the zones with `timedatectl list-timezones`. |

## Related

`clock.ntp`, `clock.set`.
