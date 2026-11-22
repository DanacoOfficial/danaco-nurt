---
command: package.upgrade
group: package
summary: Upgrade installed packages to their newest versions.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# package.upgrade

## Synopsis

```text
package.upgrade [<names>...] [manager: <enum>]
```

## Description

`package.upgrade` upgrades the named packages, or all installed packages when no name is given, through the program of the package manager. The mark [P] in the catalog identifies this use of an external program. The program is started without a shell, with the arguments as a list, and with the variables of `run.env_defaults` followed by the variables of the manager that keep it non-interactive. No program is started during a preview.

The records list the packages that the local indexes show as outdated when no name is given. pacman upgrades named packages without the rest of the system, which is a partial upgrade; the command reports a warning in that case. With no name pacman runs a full system upgrade.

The command needs the privilege of the manager. apt, dnf, pacman and zypper need the superuser and fail with `E_PRIVILEGE` otherwise; the command never calls `sudo`. Homebrew refuses to run as the superuser and fails with `E_PRIVILEGE` in that case. When the lock of the package database is held, the command fails with `E_IN_USE` and lists the holding processes.

A non-zero exit code of the manager ends the command with `E_STAGE_FAILED` and the end of its error output. The effect is reported as executed in that case, because the manager may have changed packages before it failed.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `names` | `list<text>` | none | Packages to upgrade; all installed packages when omitted. |
| `manager` | `enum(auto\|apt\|dnf\|pacman\|zypper\|brew\|winget\|choco\|scoop)` | `auto` | Package manager. `auto` selects the managers found on the system. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `package.upgrade` | none | `effect` |

The record of each package has `target` `package:<name>`, `action`, `before` (installed version, absent when the package was not installed) and `after` (version read from the package database after the run).

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `external` |
| Reversibility | `irreversible`: the journal records `package_changed` and cannot undo the change |
| External program | the manager (`apt-get`, `dnf`, `pacman`, `zypper`, `brew`, `choco`, `scoop`, `winget`) [P] |

## Examples

Upgrade all packages.

```nurt example
package.upgrade
```

Upgrade two packages.

```nurt example
package.upgrade "openssl", "curl"
```

Upgrade all packages of Scoop.

```nurt example
package.upgrade manager: scoop
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_EXTERNAL_MISSING` | The program of the manager is not installed. | Install the manager or pass another `manager`. |
| `E_PRIVILEGE` | The user lacks the privilege of the manager, or the manager refuses the superuser. | Run as root, or as a regular user for Homebrew. |
| `E_IN_USE` | The package database is locked by another process. | Wait for the listed processes to finish. |
| `E_UNSUPPORTED` | The manager cannot apply the request, for example a chosen version with pacman. | Omit the parameter. |
| `E_STAGE_FAILED` | The manager exited with an error. | Read the end of the error output in the message. |

## Related

`package.list`, `package.search`.
