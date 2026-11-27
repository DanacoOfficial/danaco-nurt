---
command: groups.remove
group: administration
summary: Remove a group.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# groups.remove

## Synopsis

```text
groups.remove <name>
```

## Description

`groups.remove` removes a group. The identifier and the members are read first and recorded. The group with identifier 0 is never removed.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` runs `groups.create` with the recorded identifier and `groups.add_member` for every recorded member.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text` | required | Group name. |

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
| Reversibility conditions | Undo creates the group again with its identifier and members. |
| External use | `system_program`: `groupdel`, `dseditgroup`, `net`; Groups are removed with groupdel on Linux, dseditgroup on macOS and net localgroup on Windows. |

## Examples

Remove a group.

```nurt example
groups.remove "ops" privilege: elevated
```

Remove a group and show the effect.

```nurt example
groups.remove "ops" privilege: elevated |> select target before
```

Run the removal with elevated rights; adding the call parameter `preview: true` shows the effect first.

```nurt example
groups.remove "ops" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | The group does not exist. | Check the name with `accounts kind: group`. |
| `E_INVALID_VALUE` | The group has identifier 0. | The group cannot be removed. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`groups.create`, `groups.remove_member`.
