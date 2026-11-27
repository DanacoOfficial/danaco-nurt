---
command: groups.create
group: admin
summary: Create a group.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# groups.create

## Synopsis

```text
groups.create <name> [name: value]...
```

## Description

`groups.create` creates a group with `groupadd` on Linux, `dseditgroup` on macOS and `net localgroup` on Windows. `gid` fixes the identifier and `system` creates a system group on Linux.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` runs `groups.remove`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text` | required | Group name. |
| `gid` | `int` | none | Numeric identifier; the system chooses one when omitted. |
| `system` | `bool` | `false` | Create a system group. |

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
| External use | `system_program`: `groupadd`, `dseditgroup`, `net`; Groups are created with groupadd on Linux, dseditgroup on macOS and net localgroup on Windows. |

## Examples

Create a group.

```nurt example
groups.create "ops" privilege: elevated
```

Create a group with an identifier.

```nurt example
groups.create "ops" gid: 3000 privilege: elevated
```

Create a system group.

```nurt example
groups.create "svc" system: true privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_EXISTS` | The group exists. | Use `groups.add_member`. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`groups.remove`, `groups.add_member`, `accounts`.
