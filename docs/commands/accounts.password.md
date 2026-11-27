---
command: accounts.password
group: admin
summary: Set the password of an account.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# accounts.password

## Synopsis

```text
accounts.password <name> <value>
```

## Description

`accounts.password` sets the password of an account. `value` is a secret parameter: it is masked in the journal, the report and the printed tree, it is written to the standard input of `chpasswd` on Linux and to an environment variable of `Set-LocalUser` on Windows, and it never appears in an argument list of Linux programs. On macOS `dscl -passwd` takes the value as an argument. The previous password is not recorded, so the change is irreversible and the report says so.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

A password is irreversible: `undo` reports that no inverse exists.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text` | required | Account name. |
| `value` | `text` | required | New password; masked in the journal and the report. |

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
| Reversibility conditions | The previous password is not recorded. |
| External use | `system_program`: `chpasswd`, `dscl`, `powershell`; The password is passed to chpasswd on its standard input on Linux, and in an environment variable of Set-LocalUser on Windows. |

## Examples

Set a password.

```nurt example
accounts.password "deploy" "correct horse battery" privilege: elevated
```

Set a password with the named parameter.

```nurt example
accounts.password name: "ann" value: "s3cret" privilege: elevated
```

Set a password and check the account afterwards.

```nurt example
accounts.password "ann" "s3cret" privilege: elevated |> select target
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | The account does not exist. | Create it with `accounts.create`. |
| `E_INVALID_VALUE` | The password holds a control character. | Use another password. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`accounts.create`, `accounts.modify`.
