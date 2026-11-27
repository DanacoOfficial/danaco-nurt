---
command: package.list
group: package
summary: List installed packages.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# package.list

## Synopsis

```text
package.list [name: value]...
```

## Description

`package.list` reports the installed packages of the package managers found on the system. It starts a pipeline and accepts no input. The command reads the package databases and the local repository indexes and starts no program; the mark [P] of the catalog does not apply to it.

| Manager | Database read |
|---|---|
| apt | `/var/lib/dpkg/status` and `/var/lib/dpkg/status.d/` |
| dnf, zypper | RPM database: `rpmdb.sqlite` or `Packages.db` in `/usr/lib/sysimage/rpm` or `/var/lib/rpm` |
| pacman | `local/` below `DBPath` of `/etc/pacman.conf`, by default `/var/lib/pacman` |
| brew | `Cellar` and `Caskroom` below `HOMEBREW_PREFIX`, `/opt/homebrew`, `/usr/local` or `/home/linuxbrew/.linuxbrew` |
| winget | installation database of App Installer below `LOCALAPPDATA` and the uninstall keys of the registry |
| scoop | `apps` below `SCOOP` or `~\scoop`, and below `SCOOP_GLOBAL` |
| choco | `lib` below `ChocolateyInstall`, by default `C:\ProgramData\chocolatey` |

`name` keeps the packages whose name contains the text, without regard to case. With `outdated: true` the command reads the local repository indexes of the manager and keeps the packages for which the newest offered version of the matching architecture is greater than the installed version; `available_version` holds that version. Versions are compared by the rules of the manager: Debian policy for apt, `rpmvercmp` for dnf and zypper, `vercmp` for pacman, version with revision for Homebrew, and numeric and alphabetic segments for winget, Scoop and Chocolatey.

When several managers are found and `manager` is `auto`, each package record names its manager. Pins and priorities of apt are not evaluated. Packages of a phased update below 100 percent are not offered as newer versions. Indexes that are missing or damaged are reported as warnings and skipped; the command fails with `E_FORMAT` only when `manager` is given and every index of that manager is unreadable. Chocolatey keeps no local index, so `outdated` finds nothing for it.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text` | none | Only packages whose name contains this text. |
| `outdated` | `bool` | `false` | Only packages with a newer version available. |
| `manager` | `enum(auto\|apt\|dnf\|pacman\|zypper\|brew\|winget\|choco\|scoop)` | `auto` | Package manager; auto selects the manager of the system. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `package` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `package_manager` |

## Examples

List the installed packages whose name contains `ssl`.

```nurt example
package.list name: "ssl"
```

List the packages that have a newer version in the repositories.

```nurt example
package.list outdated: true
```

List the packages of one manager only.

```nurt example
package.list manager: pacman |> sort by: name
```

A response that leaves records out ends with the hint "more: next" and carries the field
`continuation`; the command `next` returns the next part without running the
command again.

```text
matched 1210  shown 50  more: next
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_EXTERNAL_MISSING` | No supported package manager was found, or the given one is not installed. | Pass `manager` of an installed manager. |
| `E_FORMAT` | Every repository index of the given manager is unreadable. | Refresh the indexes with the manager. |
| `E_NOT_FOUND` | The package database of the manager is missing. | Pass `manager` of an installed manager. |
| `E_PERMISSION` | The package database cannot be opened by the account. | Run with an account that may read the database. |
| `E_FORMAT` | The package database is damaged. | Repair the database with the manager. |
| `E_LIMIT` | The packages exceed the record limit. | Narrow the list with `name`. |

## Related

`package.search`, `package.upgrade`.
