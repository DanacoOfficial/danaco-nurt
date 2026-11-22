---
name: windows
description: Paths, registry, links, services and encodings on Windows.
commands: [list, read, write, create, convert, services, run, package.install, windows.cim, info]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# windows

## Paths

A path is a quoted absolute text with `/` as the separator on every system: `"C:/proj/app"`. A backslash is not a separator in a Nurt path. Relative paths are allowed only with the call parameter `base`. A path that is valid on another system but not on the current one fails with `E_PATH_FOREIGN`.

```nurt
list "C:/Users/dev" depth: 1
info "C:/proj/app/readme.txt"
```

## Reserved names and files in use

Names such as `CON`, `NUL` and `COM1` are reserved by the system and cannot be created. A file that another process holds open cannot be replaced or removed: the command fails with `E_IN_USE`, and `open_files` names the process that holds the file.

## Links and the registry

`create` with `type: junction` and `target` creates a junction, and `type: link` creates a symbolic link. The registry is addressed as a path with the prefix `registry:` and works in `list`, `find`, `read`, `write`, `remove`, `copy` and `move`.

```nurt
create "C:/proj/current" type: junction target: "C:/proj/release-12"
read "registry:/HKCU/Software/Example"
```

## Services and packages

`services` lists services with their state, and `start`, `restart`, `enable` and `disable` change them. `package.install` with `manager: winget`, `choco` or `scoop` installs packages through the package manager; the installation is recorded as irreversible. `windows.cim` queries CIM classes.

```nurt
services name: "Spooler" |> restart
package.install ["jq"] manager: winget
windows.cim "Win32_LogicalDisk"
```

## Encodings and command interpreters

Files in code pages such as 852 or 1250 are read with `encoding` and converted with `convert`. Programs of the command interpreter or of PowerShell run through `run` with an argument list, for example `run ["powershell", "-NoProfile", "-File", "C:/ops/task.ps1"]`.

```nurt
convert "C:/legacy/readme.txt" encoding: "utf-8" line_ending: crlf
```
