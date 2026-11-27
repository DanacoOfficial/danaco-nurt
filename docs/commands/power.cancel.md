---
command: power.cancel
group: admin
summary: Cancel a scheduled restart or shutdown.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# power.cancel

## Synopsis

```text
power.cancel
```

## Description

`power.cancel` cancels a scheduled restart or shutdown with `shutdown -c` on Linux, `shutdown /a` on Windows and by stopping `shutdown` on macOS. On Linux the schedule is read from `/run/systemd/shutdown/scheduled`; without one the command reports `unchanged`. A cancelled schedule is not restored by `undo`.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` reports that the cancelled schedule is not restored.

## Parameters

The command has no parameters.

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
| Implementation | `system_program` |
| Reversibility conditions | A cancelled schedule is not restored. |
| External use | `system_program`: `shutdown`, `pkill`; The schedule is cancelled with shutdown -c on Linux, shutdown /a on Windows and by stopping shutdown on macOS. |

## Examples

Cancel the schedule.

```nurt example
power.cancel privilege: elevated
```

Preview the cancellation.

```nurt example
power.cancel preview: true
```

Cancel and show the effect.

```nurt example
power.cancel privilege: elevated |> select target
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_STAGE_FAILED` | Nothing is scheduled (Windows). | No action is needed. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`power.restart`, `power.shutdown`.
