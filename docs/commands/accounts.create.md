---
command: accounts.create
group: administration
summary: Create a user account.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/accounts.create.md -->
<!-- Responsibility:   Documents accounts.create. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# accounts.create

## Synopsis

```text
accounts.create <name> [name: value]...
```

## Description

`accounts.create` creates a user account with `useradd` on Linux, `sysadminctl` on macOS and `net user` on Windows. The account is read first: an existing name is `E_EXISTS`. `groups` lists the groups other than the primary one, `create_home: false` or `system: true` leaves the home directory out. The report records the state after the change (`uid`, `shell`, `home`, `groups`, `comment`, `locked`) as the `after` of the target `account:<name>`.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` runs `accounts.remove name: "<name>" keep_home: true`; the home directory is kept.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text` | required | Account name. |
| `uid` | `int` | none | Numeric identifier; the system chooses one when omitted. |
| `shell` | `text` | none | Login shell. |
| `home` | `text` | none | Home directory. |
| `groups` | `list<text>` | none | Groups other than the primary one. |
| `comment` | `text` | none | Comment or full name. |
| `system` | `bool` | `false` | Create a system account without a home directory. |
| `create_home` | `bool` | `true` | Create the home directory. |

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
| Reversibility conditions | Undo removes the account and keeps its home directory. |
| External use | `system_program`: `useradd`, `sysadminctl`, `net`; Accounts are created with useradd on Linux, sysadminctl on macOS and net user on Windows. |

## Examples

Create an account with a shell and a group.

```nurt example
accounts.create "deploy" shell: "/bin/bash" groups: ["docker"] privilege: elevated
```

Create a system account without a home directory.

```nurt example
accounts.create "svc-backup" system: true privilege: elevated
```

Create an account with a fixed identifier and a comment.

```nurt example
accounts.create "ann" uid: 2001 comment: "Ann Smith" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_EXISTS` | The account exists. | Use `accounts.modify`. |
| `E_INVALID_VALUE` | The name or a text starts with `-`, is empty or holds a control character, `:` or `/`. | Give a valid name. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`accounts.modify`, `accounts.remove`, `groups.create`, `accounts`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
