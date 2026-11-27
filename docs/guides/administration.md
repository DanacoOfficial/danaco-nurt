---
name: administration
description: Inspecting and administering a system: services, logs, processes, packages, accounts and disks.
commands: [services, logs, processes, ports, disks, host, environment, accounts, package.list, package.install, windows.cim, stop]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/administration.md -->
<!-- Responsibility:   Documents administration. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# administration

## Boundaries

Administration commands read the state of the system and, in some cases, change it. Changes need the rights of the administrator more often than other work, so the call parameter `privilege` is part of the language, and changes outside the file system have inverse operations.

## Elevated work

Work that needs the rights of the administrator takes the call parameter `privilege`: `account` (the default) runs with the rights of the account, `elevated` runs with those of the administrator, and `auto` runs with the account and repeats the whole stage once with `elevated` when a target fails with `E_PERMISSION` or `E_PRIVILEGE`, before any change is kept. `as_user: "NAME"` runs the operation as another account on Linux and macOS. Both are written on a stage (`write "/etc/hosts" "..." privilege: elevated`), given in the request, or set on the command line with `--privilege` and `--as-user`; the configuration key `privilege.default` sets the value for calls that give none. Nurt starts `sudo`, `doas` or `run0` (or runs in the same process when it already is the administrator), sends the syntax tree of the program, never shell text, and never asks for a password: a backend that would ask is reported as `password_required`. The report of the operation states `elevated: true`, the `backend` and, for `auto`, the `elevation_reason`. When no backend can run the operation, the call ends with `E_PRIVILEGE`; its detail `backend` lists every backend with its result and `next_steps` names the way forward. The embedding engine switches elevation off with `privilege.allowed = false`.

## Elevated entries in the journal

Elevated entries are in the journal of the account, and the account has access to their copies; `undo N` and `redo N` of such an entry restore the targets through the same backend; `undo` and `redo` without a number that would restore an elevated entry end with `E_PRIVILEGE` and the next step `undo N privilege: elevated`. The elevated process writes the journal of the account with the rights of the account on Linux and macOS; on Windows the backend `self` is the same account.

## Undo of changes outside the file system

Changes outside the file system are undone by inverse operations that the engine derives from the state recorded in the report:

| Operation | Undo |
|---|---|
| `package.install` | `package.remove` when the package was not installed before; reinstall of the recorded version when the version changed |
| `package.remove` | `package.install` of the recorded version, when the repository still offers it |
| `package.upgrade` | install of the recorded versions, when the repository still offers them |
| `start`, `stop` | the opposite command |
| `enable`, `disable` | the opposite command with the recorded start type |

`enable` and `disable` are undone only for the start types `disabled` and `automatic`; other start types make the entry irreversible, with the reason in the report. A package version that the repository no longer offers makes only that package irreversible.

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

## Native administration commands

Every command below changes the system, accepts `privilege`, reports its effect and records the previous state in the journal. The inverse is the operation that `undo` runs. The commands are loaded with the group `administration` of the tool description, so a session that does not administer a system does not carry them.

| Group | Commands | Systems | Inverse |
|---|---|---|---|
| accounts | `accounts.create`, `accounts.modify` (shell, home, groups, comment, lock), `accounts.remove` (`keep_home: bool`), `accounts.password` | Linux, macOS, Windows | create and remove are inverse (the home directory is kept until the entry expires); `modify` restores the recorded values; `password` is irreversible |
| groups | `groups.create`, `groups.remove`, `groups.add_member`, `groups.remove_member` | Linux, macOS, Windows | pairwise |
| mounts | `mount` (device, target, type, options, `persistent`), `unmount` | Linux, macOS; Windows drive letters and folder mounts | `mount` and `unmount` |
| kernel | `sysctl.set` (key, value, `persistent`), `modules.load`, `modules.unload` | Linux; `sysctl` of writable keys on macOS | the previous value; load and unload |
| power | `power.restart`, `power.shutdown` (`at`, `delay`, `message`), `power.cancel` | all | irreversible once executed; a scheduled one is undone by `power.cancel` |
| disks | `disks.partition` (create, delete, resize), `disks.format` (file system, label) | Linux, macOS, Windows | irreversible |

## Network, clock and Windows management

The commands of the network, the clock, the services and the Windows management objects follow the same rules: they report their effect, record the previous state and accept `privilege`.

| Group | Commands | Systems | Inverse |
|---|---|---|---|
| firewall | `firewall.rules` (read), `firewall.allow`, `firewall.deny`, `firewall.remove` (port, protocol, source, direction, zone) | nftables, firewalld and ufw on Linux, pf on macOS, Windows Defender Firewall | the added rule is removed |
| network | `network.route_add`, `network.route_remove`, `network.dns_set` (servers, search domains) | all | the previous routes and DNS |
| clock | `clock.timezone_set`, `clock.set`, `clock.ntp` (on, off) | all | the previous time zone and NTP state; `clock.set` is irreversible |
| services | `start`, `stop`, `restart`, `enable`, `disable` | all | as in the table above |
| Windows management | `windows.cim` method calls (`invoke: <method>`) | Windows | irreversible unless the method has a documented opposite |

## Persistence and previews

A password is given as a secret reference of the engine and never as text; the journal does not store it. `mount` with `persistent: true` writes `/etc/fstab` or a systemd mount unit, and `sysctl.set` with `persistent: true` writes `/etc/sysctl.d/90-nurt.conf`; both files are journaled like any file, so `undo` restores them. The preview of `disks.partition` and `disks.format` lists the device, its partitions, file systems and mount points, and the size of the data that would be lost.

```nurt
accounts.create "deploy" shell: "/bin/bash" groups: ["www-data"] privilege: elevated
groups.add_member "www-data" "backup" privilege: elevated
```

## Several changes as one operation

A change of a system rarely is one command. `steps` runs several changing pipelines as one journal entry, all or nothing; a step marked irreversible (a restart, a program run with `run`, `power.restart`) is listed before the start and, after a later failure, is reported as not rolled back.

```nurt
steps {
    sysctl.set "net.ipv4.ip_forward" "1" persistent privilege: elevated
    firewall.allow port: 8443 protocol: tcp privilege: elevated
    let r = run ["systemctl", "reload", "app"] cwd: "/" privilege: elevated
    assert r.exit_code == 0
}
```

`when` chooses a branch by the state of the system, and `each` repeats a change over a list; `preview` shows the chosen branch and every iteration without a change. The syntax is described in [language](../site/language.md).

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
