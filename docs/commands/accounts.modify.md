---
command: accounts.modify
group: administration
summary: Change the shell, home, groups, comment or lock state of an account.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/accounts.modify.md -->
<!-- Responsibility:   Documents accounts.modify. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# accounts.modify

## Synopsis

```text
accounts.modify <name> [name: value]...
```

## Description

`accounts.modify` changes the login shell, the home directory, the groups, the comment or the lock state of an account. `groups` is the complete list of groups other than the primary one. A call that gives the values the account already has changes nothing and reports `unchanged`. The report records the whole state before and after, so that `undo` restores the fields that changed.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` runs `accounts.modify` with the recorded values of the fields that changed.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text` | required | Account name. |
| `shell` | `text` | none | New login shell. |
| `home` | `text` | none | New home directory. |
| `groups` | `list<text>` | none | Complete list of groups other than the primary one. |
| `comment` | `text` | none | New comment or full name. |
| `lock` | `bool` | none | Lock the account when true and unlock it when false. |

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
| Reversibility conditions | Undo restores the recorded values of the fields that changed. |
| External use | `system_program`: `usermod`, `dscl`, `net`; Accounts are changed with usermod on Linux, dscl on macOS and net user on Windows. |

## Examples

Change the shell of an account.

```nurt example
accounts.modify "deploy" shell: "/bin/zsh" privilege: elevated
```

Lock an account.

```nurt example
accounts.modify "ann" lock: true privilege: elevated
```

Replace the groups of an account.

```nurt example
accounts.modify "deploy" groups: ["docker", "wheel"] comment: "Deploy user" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | The account does not exist. | Create it with `accounts.create`. |
| `E_MISSING_PARAM` | No field to change was given. | Give at least one of `shell`, `home`, `groups`, `comment`, `lock`. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`accounts.create`, `accounts.remove`, `accounts`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
