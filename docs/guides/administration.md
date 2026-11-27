---
name: administration
description: Inspecting and administering a system: services, logs, processes, packages, accounts and disks.
commands: [services, logs, processes, ports, disks, host, environment, accounts, package.list, package.install, windows.cim, stop]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# administration

## Boundaries

Administration commands read the state of the system and, in some cases, change it.

Work that needs the rights of the administrator takes the call parameter `privilege`: `account` (the default) runs with the rights of the account, `elevated` runs with those of the administrator, and `auto` runs with the account and repeats the whole stage once with `elevated` when a target fails with `E_PERMISSION` or `E_PRIVILEGE`, before any change is kept. `as_user: "NAME"` runs the operation as another account on Linux and macOS. Both are written on a stage (`write "/etc/hosts" "..." privilege: elevated`), given in the request, or set on the command line with `--privilege` and `--as-user`; the configuration key `privilege.default` sets the value for calls that give none. Nurt starts `sudo`, `doas` or `run0` (or runs in the same process when it already is the administrator), sends the syntax tree of the program, never shell text, and never asks for a password: a backend that would ask is reported as `password_required`. The report of the operation states `elevated: true`, the `backend` and, for `auto`, the `elevation_reason`. When no backend can run the operation, the call ends with `E_PRIVILEGE`; its detail `backend` lists every backend with its result and `next_steps` names the way forward. The embedding engine switches elevation off with `privilege.allowed = false`.

Elevated changes are written to the system journal (`/var/lib/nurt` on Linux, `/Library/Application Support/Nurt` on macOS, `%ProgramData%\Nurt` on Windows). The account keeps a reference to every entry, so `undo` with the number of the entry reaches it through the same backend.

Changes outside the file system are undone by inverse operations that the engine derives from the state recorded in the report:

| Operation | Undo |
|---|---|
| `package.install` | `package.remove` when the package was not installed before; reinstall of the recorded version when the version changed |
| `package.remove` | `package.install` of the recorded version, when the repository still offers it |
| `package.upgrade` | install of the recorded versions, when the repository still offers them |
| `start`, `stop` | the opposite command |
| `enable`, `disable` | the opposite command with the recorded start type |

Irreversible are `restart`, `stop` of a process, and `run`: files changed by a program started with `run` and `privilege: elevated` are not preserved, and the report states it.

## Services

`services` lists services with their state and start type. `state` narrows the list and `scope` selects system or user services. `start`, `restart`, `enable` and `disable` take services from the input, and `stop` ends services, processes and jobs.

```nurt
services state: running
services name: "nginx" |> restart
```

## Processes and ports

`processes` lists running processes, and `ports` lists ports with the processes that hold them. `open_files` names the processes that hold a file open, which explains `E_IN_USE`.

```nurt
processes name: "node" |> filter memory > 500mb |> select pid name memory
ports state: listen |> filter port == 8080
```

## Logs

`logs` reads events from the system log: the systemd journal, the Windows Event Log or the macOS unified log. `level` sets the lowest severity, `since` and `until` bound the time, and `message` filters the text.

```nurt
logs level: error since: ago(1h) limit: 50
```

## Packages and accounts

`package.list` and `package.search` read the package repositories, and `package.install`, `package.remove` and `package.upgrade` change installed packages through the manager of the system. `accounts` lists user accounts and groups, `host` describes the operating system and `environment` lists variables with the scope in which they are set. On Windows `windows.cim` queries CIM classes.

```nurt
package.list outdated: true
accounts kind: group
host
```

## Disks

`disks` lists mounted volumes with their capacity and free space. The `journal.space` command reports how much of that space the journal holds.

```nurt
disks |> filter free < 10gib
```
