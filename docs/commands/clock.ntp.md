---
command: clock.ntp
group: administration
summary: Switch network time synchronisation on or off.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/clock.ntp.md -->
<!-- Responsibility:   Documents clock.ntp. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# clock.ntp

## Synopsis

```text
clock.ntp <state>
```

## Description

`clock.ntp` switches synchronisation with network time on or off: `timedatectl set-ntp` on Linux, `systemsetup -setusingnetworktime` on macOS and the service `w32time` on Windows. A state that the system already has changes nothing. `undo` restores the previous state.

The previous value is read before the change and recorded as the target `clock:ntp`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `state` | `enum(on\|off)` | required | Synchronise the clock with network time or not. |

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
| Reversibility conditions | `undo` restores the previous state. |
| External use | `system_program`: `timedatectl`, `systemsetup`, `tzutil`, `date`, `powershell`; The clock is read and set through timedatectl, systemsetup, tzutil, date and, on Windows, PowerShell for the time service. |

## Examples

Switch network time on.

```nurt example
clock.ntp on privilege: elevated
```

Switch network time off.

```nurt example
clock.ntp off privilege: elevated
```

Switch network time off before setting the clock by hand.

```nurt example
clock.ntp state: off privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | `state` is neither `on` nor `off`. | Write `on` or `off`. |
| `E_PRIVILEGE` | The program of the system refused the change for want of rights. | Repeat the call with `privilege: elevated`. |

## Related

`clock.timezone_set`, `clock.set`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
