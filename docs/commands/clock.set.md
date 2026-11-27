---
command: clock.set
group: administration
summary: Set the date and time of the clock.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# clock.set

## Synopsis

```text
clock.set <time>
```

## Description

`clock.set` sets the date and time of the clock to an instant in UTC: `date -u -s` on Linux, `date -u` on macOS and `Set-Date` on Windows. Setting the time is irreversible: the previous reading is lost, so `undo` cannot restore it and the report lists the change as irreversible. A system with network time switched on corrects the clock again; switch it off first with `clock.ntp`.

The previous value is read before the change and recorded as the target `clock:time`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `time` | `instant` | required | New time; an instant in UTC. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `external` |
| Reversibility | `irreversible` |
| Implementation | `builtin` |
| Reversibility conditions | The previous reading of the clock cannot be restored. |
| External use | `system_program`: `timedatectl`, `systemsetup`, `tzutil`, `date`, `powershell`; The clock is read and set through timedatectl, systemsetup, tzutil, date and, on Windows, PowerShell for the time service. |

## Examples

Set the clock.

```nurt example
clock.set "2026-11-21T09:12:00Z" privilege: elevated
```

Set the clock to midnight UTC.

```nurt example
clock.set "2027-01-01T00:00:00Z" privilege: elevated
```

Set the clock to the last minute of the year.

```nurt example
clock.set "2026-12-31T23:59:00Z" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_TYPE` | `time` is not an instant. | Write an instant such as `2026-11-21T09:12:00Z`. |
| `E_PRIVILEGE` | The program of the system refused the change for want of rights. | Repeat the call with `privilege: elevated`. |

## Related

`clock.ntp`, `clock.timezone_set`.
