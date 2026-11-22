---
command: environment
group: system
summary: List environment variables with their values.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# environment

## Synopsis

```text
environment [<name>] [name: value]...
```

## Description

`environment` lists environment variables as `environment_variable` records
with `name`, `value` and `scope`. Values are listed as they are and are never
masked.

`name` selects variables. Text selects one variable by its name, compared
case-sensitively on Linux and macOS and regardless of case on Windows. A
pattern selects every variable whose name matches it.

| `scope` | Variables | Systems |
|---|---|---|
| `process` | environment of the console process | all |
| `user` | persistent variables of the account, `registry:/HKCU/Environment` | Windows |
| `machine` | persistent variables of the computer, `registry:/HKLM/SYSTEM/CurrentControlSet/Control/Session Manager/Environment` | Windows |

Persistent variables of type `REG_EXPAND_SZ` are listed without expanding
references such as `%USERPROFILE%`. A value that is not valid UTF-8 is listed
with the invalid bytes replaced by U+FFFD; the report carries one
`encoding_changed` note for each such variable.

Linux and macOS have no persistent variables outside profile files, so
`scope: user` and `scope: machine` fail there with `E_OS_ONLY`. Persistent
variables are changed with `write` and `replace`: on Windows by writing the
registry value, which also notifies running programs with
`WM_SETTINGCHANGE`; on Linux and macOS by editing a profile file such as
`~/.profile` or `~/.zshenv`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text \| regex` | none | Variable name or pattern. |
| `scope` | `enum(process\|user\|machine)` | `process` | Variables of this process, or persistent user or machine variables on Windows. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `environment_variable` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Show the search path of the console process:

```nurt example
environment "PATH"
```

```text expected
name: PATH
scope: process
```

List the variables whose names start with `CARGO_`:

```nurt example
environment /^CARGO_/
```

List the persistent variables of the account on Windows:

```nurt example
environment scope: user
```

Set a persistent variable of the account on Windows:

```nurt example
write "registry:/HKCU/Environment/EDITOR" "code" mode: overwrite value_type: string
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_OS_ONLY` | `scope: user` or `scope: machine` outside Windows. | Use `scope: process`, or edit a profile file with `write` or `replace`. |
| `E_REGEX` | `name` is an invalid pattern. | Correct the pattern. |
| `E_TYPE` | `name` is neither text nor a pattern, or `scope` is not a listed value. | Pass a name, a pattern or one of the listed scopes. |
| `E_PRIVILEGE` | The registry key of the scope cannot be read. | Run the console with an account that can read the key. |
| `E_IO` | The registry key of the scope cannot be read for another reason. | Check the key with `list "registry:/HKCU/Environment"`. |

## Related

`host`, `write`, `replace`, `list`.
