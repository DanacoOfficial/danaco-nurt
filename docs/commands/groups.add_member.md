---
command: groups.add_member
group: administration
summary: Add an account to a group.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/groups.add_member.md -->
<!-- Responsibility:   Documents groups.add_member. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# groups.add_member

## Synopsis

```text
groups.add_member <name> <member>
```

## Description

`groups.add_member` adds an account to a group with `gpasswd -a` on Linux, `dseditgroup` on macOS and `net localgroup /add` on Windows. An account that already belongs to the group is reported as `unchanged`.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` runs `groups.remove_member`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text` | required | Group name. |
| `member` | `text` | required | Account to add. |

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
| External use | `system_program`: `gpasswd`, `dseditgroup`, `net`; Members are added with gpasswd on Linux, dseditgroup on macOS and net localgroup on Windows. |

## Examples

Add an account to a group.

```nurt example
groups.add_member "docker" "deploy" privilege: elevated
```

Add an account named by parameter.

```nurt example
groups.add_member name: "ops" member: "ann" privilege: elevated
```

Run the change with elevated rights; adding the call parameter `preview: true` shows the effect first.

```nurt example
groups.add_member "ops" "ann" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | The group does not exist. | Create it with `groups.create`. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`groups.remove_member`, `groups.create`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
