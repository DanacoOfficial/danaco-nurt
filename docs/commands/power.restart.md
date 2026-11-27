---
command: power.restart
group: administration
summary: Restart the system now or at a given time.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/power.restart.md -->
<!-- Responsibility:   Documents power.restart. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# power.restart

## Synopsis

```text
power.restart [name: value]...
```

## Description

`power.restart` restarts the system with `shutdown`. Without `at` and `delay` the action runs at once and is irreversible; the report says so before the change. With `at` (an instant, converted to a delay with the clock of the engine) or `delay` (rounded up to whole minutes on Linux and macOS) the action is scheduled and `power.cancel` undoes it. A second schedule while one exists is `E_IN_USE` on Linux. `message` is shown to the logged-in users.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` of a scheduled action runs `power.cancel`; an immediate action cannot be undone.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `at` | `instant` | none | Time of the restart; converted to a delay with the clock of the engine. |
| `delay` | `duration` | none | Time until the restart; rounded up to whole minutes on Linux and macOS. |
| `message` | `text` | none | Message shown to the logged-in users. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `external` |
| Reversibility | `reversible_if_preserved` |
| Implementation | `system_program` |
| Reversibility conditions | A scheduled restart is undone by power.cancel; one without at or delay runs at once and is irreversible. |
| External use | `system_program`: `shutdown`; The system is restarted with the program shutdown. |

## Examples

Schedule in ten minutes.

```nurt example
power.restart delay: 10m message: "Maintenance" privilege: elevated
```

Schedule at a time.

```nurt example
power.restart at: "2026-12-01T03:00:00Z" privilege: elevated
```

Run the immediate action with elevated rights; adding the call parameter `preview: true` shows the effect first.

```nurt example
power.restart privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_IN_USE` | A restart or shutdown is already scheduled. | Cancel it with `power.cancel`. |
| `E_INVALID_VALUE` | `at` is in the past. | Give a later time. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`power.cancel`, `power.restart`, `power.shutdown`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
