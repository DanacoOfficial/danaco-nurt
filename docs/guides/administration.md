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

Administration commands read the state of the system and, in some cases, change it. A change to a service, a process or a package is outside the file system, so the journal records it as irreversible and cannot undo it. Commands that need privileges that the console does not hold fail with `E_PRIVILEGE`; the console does not raise privileges.

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
