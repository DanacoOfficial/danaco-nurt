---
command: accounts.remove
group: admin
summary: Remove a user account.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# accounts.remove

## Synopsis

```text
accounts.remove <name> [name: value]...
```

## Description

`accounts.remove` removes an account with `userdel` on Linux, `sysadminctl` on macOS and `net user` on Windows. The home directory is not deleted by the system program: unless `keep_home` is true it is removed through the journal, so `undo` brings it back with its content. The account with identifier 0 is never removed. The report records the whole state of the account before the change.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` runs `accounts.create` with the recorded identifier, shell, home, groups and comment (`create_home: false`), then `accounts.modify` for a locked account; the journal restores the home directory.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text` | required | Account name. |
| `keep_home` | `bool` | `false` | Keep the home directory; otherwise it is removed through the journal. |

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
| Reversibility conditions | Undo creates the account again with its identifier, shell, groups and comment; the home directory returns with the journal. |
| External use | `system_program`: `userdel`, `sysadminctl`, `net`; Accounts are removed with userdel on Linux, sysadminctl on macOS and net user on Windows. |

## Examples

Remove an account and its home directory.

```nurt example
accounts.remove "olduser" privilege: elevated
```

Remove an account and keep the home directory.

```nurt example
accounts.remove "olduser" keep_home privilege: elevated
```

Preview the removal.

```nurt example
accounts.remove "olduser" preview
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | The account does not exist. | Check the name with `accounts`. |
| `E_INVALID_VALUE` | The account is the superuser. | The superuser cannot be removed. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`accounts.create`, `accounts.modify`, `accounts`.
