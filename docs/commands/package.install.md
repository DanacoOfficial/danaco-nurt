---
command: package.install
group: package
summary: Install packages.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/package.install.md -->
<!-- Responsibility:   Documents package.install. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# package.install

## Synopsis

```text
package.install <names>... [name: value]...
```

## Description

`package.install` installs the named packages through the program of the package manager. The mark [P] in the catalog identifies this use of an external program. The program is started without a shell, with the arguments as a list, and with the variables of `run.env_defaults` followed by the variables of the manager that keep it non-interactive. No program is started during a preview.

With `version` every named package is installed in that version: `name=version` for apt and zypper, `name-version` for dnf, `name@version` for Homebrew and Scoop, and `--version` for Chocolatey and winget. pacman installs the version of the repository only and rejects `version` with `E_UNSUPPORTED`. An installed package is kept by pacman (`--needed`).

`before` comes from the package database read before the run, `after` from the database read again after it. The engine keeps the state before the change, and `undo` of the entry restores it (see Privilege).

The command needs the privilege of the manager. apt, dnf, pacman and zypper need the superuser and fail with `E_PRIVILEGE` otherwise; with `privilege: elevated` the call runs through an elevation backend (`sudo`, `doas`, `run0`, or in the same process when Nurt is the administrator), and the command itself never writes `sudo`. Homebrew refuses to run as the superuser and fails with `E_PRIVILEGE` in that case. When the lock of the package database is held, the command fails with `E_IN_USE` and lists the holding processes.

A non-zero exit code of the manager ends the command with `E_STAGE_FAILED` and the end of its error output. The effect is reported as executed in that case, because the manager may have changed packages before it failed.

**Privilege.** apt, dnf, pacman and zypper need the administrator. Write `privilege: elevated` on the stage to run the command through an elevation backend; without it the command fails with `E_PRIVILEGE`.

```nurt
package.install ["jq"] privilege: elevated
```

`undo` of the entry runs `package.remove` for a package that was not installed before, and installs the recorded version again for a package whose version the command changed.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `names` | `list<text>` | required | Packages to install. |
| `version` | `text` | none | Version to install. |
| `manager` | `enum(auto\|apt\|dnf\|pacman\|zypper\|brew\|winget\|choco\|scoop)` | `auto` | Package manager; auto selects the manager of the system. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

The record of each package has `target` `package:<name>`, `action`, `before` (installed version, absent when the package was not installed) and `after` (version read from the package database after the run).

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `external` |
| Reversibility | `irreversible` |
| Implementation | `package_manager` |

## Examples

Install one package.

```nurt example
package.install "ripgrep"
```

Install two packages with the apt manager.

```nurt example
package.install "curl" "jq" manager: apt
```

Install a chosen version of a package.

```nurt example
package.install "nginx" version: "1.24.0-2" manager: apt
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

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
