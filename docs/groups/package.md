---
group: package
summary: System package managers.
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Group package

## Scope

The group lists, searches, installs, removes and upgrades packages through the
package manager of the system and returns the result as `package` records. The
commands do not replace the package manager: it carries out every operation,
and the group gives its output a typed form and records the change in the
operation journal.

| Task | Command | Group |
|---|---|---|
| Find an installed program by name | `program.find` | program |
| Describe an installed program | `program.describe` | program |
| Search programs of all sources, including packages | `program.search` | program |
| Run a program | `run` | core |

`package.search` queries the repositories of the package manager.
`program.search` reads the package databases of the system and needs no
package manager.

## Commands

| Command | Category | Input and output | Reversibility | Page |
|---|---|---|---|---|
| `package.list` | source | none to `package` | read | [package.list](../commands/package.list.md) |
| `package.search` | source | none to `package` | read | [package.search](../commands/package.search.md) |
| `package.install` | change | none to `effect` | irreversible | [package.install](../commands/package.install.md) |
| `package.remove` | change | none to `effect` | irreversible | [package.remove](../commands/package.remove.md) |
| `package.upgrade` | change | none to `effect` | irreversible | [package.upgrade](../commands/package.upgrade.md) |

## Records

| Record | Required fields |
|---|---|
| `package` | `name`, `version`, `manager`, `installed` |
| `effect` | `status`, `kind` |

`package.version` is the installed version, or the offered version in the
result of a search. With `outdated: true` the field `available_version` holds
the newer version. Optional fields are `description`, `architecture`, `size`,
`source` and `files_count`.

## Package managers

The parameter `manager` selects the package manager. The default is `auto`:
the manager that the host uses is detected from the operating system and the
programs found in the search path.

| Value | System |
|---|---|
| `apt` | Debian and Ubuntu |
| `dnf` | Fedora and Red Hat family |
| `pacman` | Arch Linux |
| `zypper` | openSUSE and SUSE |
| `brew` | macOS and Linux with Homebrew |
| `winget` | Windows |
| `choco` | Windows with Chocolatey |
| `scoop` | Windows with Scoop |

The record field `manager` also accepts `dpkg`, `rpm` and `other` for packages
that the manager reports with another origin. A value of `manager` that the
host does not have fails with `E_EXTERNAL_MISSING`.

Installation, removal and upgrade change the system outside the file system.
The operation journal records them as irreversible: `undo` does not restore a
removed package and does not remove an installed one.
## Platform differences

| Aspect | Linux | macOS | Windows |
|---|---|---|---|
| Managers | `apt`, `dnf`, `pacman`, `zypper`, `brew` | `brew` | `winget`, `choco`, `scoop` |
| Privilege for `install`, `remove`, `upgrade` | the privilege that the manager requires | the privilege that the manager requires | the privilege that the manager requires |
| Missing privilege | `E_PRIVILEGE` | `E_PRIVILEGE` | `E_PRIVILEGE` |

A command whose manager is not installed fails with `E_EXTERNAL_MISSING`, and
`fix` shows how to install it.

## External programs

The catalog marks the implementation of every command of the group as a
package manager program [P]:

| Command | Marker | Program |
|---|---|---|
| `package.list` | [P] | the selected manager |
| `package.search` | [P] | the selected manager |
| `package.install` | [P] | the selected manager |
| `package.remove` | [P] | the selected manager |
| `package.upgrade` | [P] | the selected manager |

The check of the program reports the program before execution.

## Examples

List the installed packages named `git`:

```nurt example
package.list name: "git"
```

Count the packages that have a newer version:

```nurt example
package.list outdated: true |> count
```

Show the installed and the available version of outdated packages:

```nurt example
package.list outdated: true |> select name version available_version
```

Search the repositories of the manager:

```nurt example
package.search "ripgrep"
```

Install two packages with the manager `apt`:

```nurt example
package.install ["git", "curl"] manager: apt
```

Upgrade one package:

```nurt example
package.upgrade ["openssl"]
```

## Related

[Group program](program.md) describes installed programs without a package
manager. [Group system](system.md) reports the host on which `manager: auto`
decides.
