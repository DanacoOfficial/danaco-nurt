---
command: groups.remove_member
group: admin
summary: Remove an account from a group.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# groups.remove_member

## Synopsis

```text
groups.remove_member <name> <member>
```

## Description

`groups.remove_member` removes an account from a group with `gpasswd -d` on Linux, `dseditgroup` on macOS and `net localgroup /delete` on Windows. An account that is not a member is reported as `unchanged`.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` runs `groups.add_member`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text` | required | Group name. |
| `member` | `text` | required | Account to remove. |

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
| External use | `system_program`: `gpasswd`, `dseditgroup`, `net`; Members are removed with gpasswd on Linux, dseditgroup on macOS and net localgroup on Windows. |

## Examples

Remove an account from a group.

```nurt example
groups.remove_member "docker" "deploy" privilege: elevated
```

Remove an account named by parameter.

```nurt example
groups.remove_member name: "ops" member: "ann" privilege: elevated
```

Preview the change.

```nurt example
groups.remove_member "ops" "ann" preview: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | The group does not exist. | Check the name with `accounts kind: group`. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`groups.add_member`, `groups.remove`.
