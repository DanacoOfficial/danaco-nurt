---
command: accounts
group: system
summary: List user accounts and groups.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# accounts

## Synopsis

```text
accounts [kind: user|group|all] [name: text]
```

## Description

`accounts` lists the local user accounts and groups as `account` records.
Users come first, in the order of the account database, followed by groups.

| Field | Users | Groups |
|---|---|---|
| `id` | UID; on Windows the security identifier | GID; on Windows the security identifier |
| `groups` | primary group, then the groups that list the user as a member | members |
| `home`, `shell` | home directory and login shell | not set |
| `full_name` | first part of the comment field; on Windows the full name | not set |
| `disabled` | locked password on Linux; `UF_ACCOUNTDISABLE` on Windows | not set |
| `last_login` | last logon on Windows | not set |
| `system` | account of the operating system | group of the operating system |

| System | Source | `system` |
|---|---|---|
| Linux | user and group databases of the system library, including every configured source | identifier below `UID_MIN` of `/etc/login.defs`, 1000 when unset |
| macOS | user and group databases of the system library; the directory service supplies the local accounts | identifier below 500, or a name starting with `_` |
| Windows | local accounts and local groups | relative identifier below 1000; built-in groups |

`nobody`, `nogroup` and `nfsnobody` are system accounts on every system. On
Linux `disabled` comes from `/etc/shadow`: an account whose password field
starts with `!` or is `*` is disabled. When the file cannot be read,
`disabled` is not set and the report carries one `permission_denied_skipped`
note. macOS does not report `disabled`. Domain accounts on Windows are not
listed.

`name` selects the account or group with that name, compared
case-sensitively on Linux and macOS and regardless of case on Windows.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `kind` | `enum(user\|group\|all)` | `all` | Users, groups or both. |
| `name` | `text` | | Only the account or group with this name. |

## Input and output

| Input | Output |
|---|---|
| none | `account` |

## Effect

Category `source`, effect `read`, reversibility `read`. The command reads the
account databases and changes nothing.

## Examples

List the accounts of people:

```nurt example
accounts kind: user | where not system
```

```text expected
kind: user
system: false
```

Show the members of one group:

```nurt example
accounts kind: group name: "docker" | select groups
```

List the disabled accounts:

```nurt example
accounts kind: user | where disabled
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_TYPE` | `kind` is not a listed value, or `name` is not text. | Use `user`, `group` or `all`, and a text name. |
| `E_PRIVILEGE` | The account database refuses access. | Run the console as an administrator. |
| `E_IO` | The account database cannot be read. | Check that `/etc/passwd` and `/etc/group` exist. |

## Related

`host`, `processes`, `open_files`.
