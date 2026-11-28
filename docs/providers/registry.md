<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/providers/registry.md -->
<!-- Responsibility:   Documents registry:. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# registry:

## Synopsis

```text
registry:/<root>/<key>/…/<key>
registry:/<root>/<key>/…/<value name>
registry:/<root>/<key>/…/(default)
```

## Description

The `registry:` provider shows the Windows registry as a tree of paths. A key
is a directory and is described by an `entry` record with `type: directory`;
a value is described by a `registry_value` record. The commands `list`,
`find`, `read`, `info`, `write`, `remove`, `copy` and `move` accept
`registry:` paths in place of file paths. The provider exists only on
Windows; elsewhere every `registry:` path is rejected while binding with
`E_OS_ONLY`.

The registry is accessed through the Win32 registry functions. No external
program is started.

## Paths

| Element | Rule |
|---|---|
| Root | `HKLM`, `HKCU`, `HKCR`, `HKU` or `HKCC`. The full names `HKEY_LOCAL_MACHINE`, `HKEY_CURRENT_USER`, `HKEY_CLASSES_ROOT`, `HKEY_USERS` and `HKEY_CURRENT_CONFIG` are accepted and shown in the short form. |
| Separator | `/`. A `/` inside a key or value name is written `%2F`, and `%` is written `%25`. |
| `\` | Does not separate keys. It is accepted only in the last segment, as part of the name of a value (`registry:/HKCU/Example/C:\Tools\app.exe`). A path with `\` in a segment that names a key, or in its only segment below the root, is rejected with `E_INVALID_VALUE`: write `registry:/HKCU/Software/Example`, not `registry:/HKCU/Software\Example`. |
| Case | Names are compared without regard to case. Results show the spelling stored in the registry. |
| Last segment | Names a value when the key before it has a value of that name, otherwise a key. |
| `(default)` | Names the default value of a key, whose `name` is empty in `registry_value`. |
| View | The 64-bit view of the registry. Records carry the view in the field `view`. |

## Records

`registry_value`:

| Field | Type | Meaning |
|---|---|---|
| `path` | path | Key that holds the value. |
| `name` | text | Name; empty for the default value. |
| `type` | enum(string\|expand_string\|multi_string\|dword\|qword\|binary\|none\|other) | Type. Data whose length does not match `dword` or `qword`, and the types `REG_DWORD_BIG_ENDIAN`, `REG_LINK` and the resource lists, are shown as `other`. |
| `value` | any | Text for `string` and `expand_string` (references are not expanded), int for `dword` and `qword`, list of texts for `multi_string`, bytes for `binary`, `none` and `other`. |
| `size` | size | Size of the data in bytes. |
| `view` | enum(64\|32) | View of the registry. |

A key is an `entry` with `type: directory`, `modified` set to the time of the
last write to the key and `size` 0. A symbolic link key is an `entry` with
`type: link` and `link_target`; `find` does not enter it.

## Writing

`write "registry:/…" "<value>" mode: <mode> value_type: <type>` sets a value
and creates missing keys. `mode` is required, as for files, and is `create`
(the value must not exist) or `overwrite`. The value is the text parameter of
`write`; without `value_type` it is written as `string`. Numbers are written
as text and converted to the type.

| `value_type` | Accepted value |
|---|---|
| `string`, `expand_string` | text |
| `dword` | text with a decimal or `0x` hexadecimal number from 0 to 4294967295 |
| `qword` | text with a decimal or `0x` hexadecimal number from 0 to 18446744073709551615 |
| `multi_string` | text with one string per line |
| `binary` | bytes in the parameter `data` |

`remove` deletes a value, or a key with all its subkeys and values. A root
key cannot be removed. `copy` and `move` copy a key with its subtree, or a
value; the destination must not exist, and missing keys above it are
created. `move` is one operation of the journal.

## Commands

| Command | Behaviour on `registry:` paths |
|---|---|
| `read` | Returns the `registry_value` record of each value. A key is refused with `E_IO` (`kind: is_a_directory`): list it instead. Line selectors do not apply. |
| `info` | Returns the `entry` record of a key or the `registry_value` record of a value; a path that names nothing gives an `entry` with `exists: false`. |
| `list` | Keys first, then values, to `depth` levels (default 1). `type` selects values (`file`), keys (`directory`), link keys (`link`) or all (`any`). |
| `find` | Keys and values below a key to `depth` levels (default unlimited). `type` defaults to `file`, which selects values. `glob` is matched without regard to case against the name and against the path below the key. The key itself is a candidate for `type: any` and `type: directory` when no `glob` is given. |
| `write` | Sets a value as described in Writing. `mode` is `create` or `overwrite`. |
| `remove` | Removes the values and keys named by `path`, or by piped `entry` records of keys. A key inside a key that is removed as well is removed once, with the enclosing key. `on_missing` applies. |
| `copy`, `move` | `to` names the destination of one source, `into` the key that receives every source under its own name. Sources and destination must all be registry paths. The destination must not exist, whatever `on_conflict` says. |

`list`, `find`, `read` and `info` return records of the kinds named in Records.
`remove`, `copy` and `move` declare their targets to the journal like the
changes of files; a `move` declares the copy and the removal of the source in
one operation. A command that takes only file paths rejects a `registry:` path
with `E_INVALID_VALUE`, and a host that has no registry rejects it with
`E_OS_ONLY`.

## Effect

Every change is reversible with `undo`. Before the change the journal keeps:

| Target | Kept state |
|---|---|
| value | existence, type code and exact bytes, including text stored without a terminating NUL |
| key | existence and the whole subtree: class, values with type codes and bytes, subkeys and the discretionary access control list |
| missing keys | the depth at which the path did not exist; undo deletes the keys the operation created |

Undo restores the kept state exactly; the time of the last write of restored
keys is the time of the restoration. Volatile keys and symbolic link keys
inside a subtree are not kept: the report marks them
`irreversible: previous_content_not_kept`. A state larger than
`journal.max_op_size` is not kept and is marked
`irreversible: too_large_for_journal`; the change is still made.

A change below `HKCU/Environment` or
`HKLM/SYSTEM/CurrentControlSet/Control/Session Manager/Environment`, and its
undo, is announced to running programs with `WM_SETTINGCHANGE`, so that new
processes started by them see the changed variables. The announcement itself
cannot be undone.

## Examples

Read the product name of the installed Windows edition:

```nurt
read "registry:/HKLM/SOFTWARE/Microsoft/Windows NT/CurrentVersion/ProductName"
```

Set the persistent environment variable `EDITOR` of the current account:

```nurt
write "registry:/HKCU/Environment/EDITOR" "code" mode: overwrite value_type: string
```

Remove a key with its subtree and restore it:

```nurt
remove "registry:/HKCU/Software/Example"
undo
```

Read the default value of a key:

```nurt
read "registry:/HKCU/Software/Example/(default)"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_OS_ONLY` | the registry exists only on Windows | run the program on Windows |
| `E_NOT_FOUND` | the key or value does not exist | check the path with `list` |
| `E_EXISTS` | the destination of `copy` or `move` exists | remove the destination first |
| `E_PERMISSION` | access to the key is denied, or a root key was to be removed | run with an account that has access |
| `E_PRIVILEGE` | the change requires the `administrator` privilege | run as administrator |
| `E_TYPE` | the value does not match `value_type` | pass a value of the type listed above |
| `E_INVALID_VALUE` | a number is out of range, a text contains NUL, or the path does not parse | correct the value or the path |

## Related

`environment` reads persistent environment variables; `list`, `find`, `read`,
`info`, `write`, `remove`, `copy`, `move` and `undo` describe the commands.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
