---
name: unix
description: Permissions, services, packages and file names on Linux and macOS.
commands: [attributes, info, find, services, package.install, package.list, create, run, list]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# unix

## Permissions

`attributes` changes the mode, the account and the group of an entry, the times and the extended attributes. `recursive: true` applies the change to a directory tree. `info` reports the mode and the account of a path, so a change can be checked with `assert` in the same program.

```nurt
attributes "/srv/app/run.sh" mode: "0755"
find "/srv/app" glob: "**/*.sh" |> filter mode != "0755"
```

## Services

On Linux `services` reads systemd units, and `scope: user` selects the units of the current account instead of the system units. On macOS it reads launchd jobs. `start`, `restart`, `enable` and `disable` change the state of the input services; they need the rights of the administrator for system units (`privilege: elevated`). `start`, `stop`, `enable` and `disable` are undone by the opposite command, `restart` is irreversible.

```nurt
services name: "nginx" |> restart
services scope: user
```

## Packages

`package.install`, `package.remove` and `package.upgrade` call the package manager of the system. `manager` selects `apt`, `dnf`, `pacman` or `zypper` on Linux and `brew` on macOS; `auto` selects the manager found on the system. Installing and removing packages needs the rights of the administrator: write `privilege: elevated` (`package.install ["jq"] privilege: elevated`); without them the command fails with `E_PRIVILEGE`. `undo` removes a package that was installed, or installs the recorded version again.

```nurt
package.list outdated: true manager: apt
package.install ["jq"] manager: brew
```

## File names

Linux file systems distinguish names that differ only in case. The default volume format of macOS does not, and it stores names in a decomposed Unicode form. Two paths that look equal can therefore be different entries on one system and the same entry on another. `find` with `case_sensitive: false` matches names without regard to case.

## Links and programs

`create` with `type: link` and `target` creates a symbolic link, and `hard: true` creates a hard link. A script is started through `run` with its interpreter named in the argument list, so the result does not depend on a shell.

```nurt
create "/srv/app/current" type: link target: "/srv/app/release-12"
```
