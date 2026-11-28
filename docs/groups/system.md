---
group: system
summary: Processes, ports, disks, host, environment, system logs and suspension of processes.
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/groups/system.md -->
<!-- Responsibility:   Documents Group system. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Group system

## Scope

The group reports the state of the computer: running processes, network ports,
mounted volumes, facts about the host, environment variables, accounts, files
held open by processes and the system log. `sleep` belongs to the group
because it waits on the clock of the program and reads nothing.

Commands that act on the state reported here belong to other groups:

| Task | Command | Group |
|---|---|---|
| Stop a process, job or service | `stop` | core |
| Read the operation journal of Danaco Nurt | `journal.*` | journal |
| Read and change files, including `registry:` paths | `read`, `write`, `list` | core |
| Manage services | `services`, `start`, `restart` | administration |

`suspend` and `resume` are listed at the end of the command table. Their
pages are maintained with the commands that run and control programs.

## Commands

Every source command takes no input and produces records of one kind.

| Command | Category | Input and output | Reversibility | Page |
|---|---|---|---|---|
| `processes` | source | none to `process` | read | [processes](../commands/processes.md) |
| `ports` | source | none to `port` | read | [ports](../commands/ports.md) |
| `disks` | source | none to `disk` | read | [disks](../commands/disks.md) |
| `host` | source | none to `host` | read | [host](../commands/host.md) |
| `environment` | source | none to `environment_variable` | read | [environment](../commands/environment.md) |
| `open_files` | source | none to `process` | read | [open_files](../commands/open_files.md) |
| `accounts` | source | none to `account` | read | [accounts](../commands/accounts.md) |
| `sleep` | run | none to none | read | [sleep](../commands/sleep.md) |
| `logs` | source | none to `log_event` | read | [logs](../commands/logs.md) |

Commands of the group described by other pages:

| Command | Category | Input and output | Reversibility |
|---|---|---|---|
| `suspend` | run | `job` or `process` to `effect` | irreversible |
| `resume` | run | `job` or `process` to `effect` | irreversible |

## Records

Fields that every record of the kind carries. Optional fields are listed on
the page of the command.

| Record | Required fields |
|---|---|
| `process` | `pid`, `parent_pid`, `name`, `command`, `user`, `memory`, `started`, `state` |
| `port` | `protocol`, `local`, `address`, `port`, `state` |
| `disk` | `mount`, `device`, `filesystem`, `total`, `free`, `used`, `type`, `case_sensitive`, `readonly` |
| `host` | `os`, `os_version`, `architecture`, `cpus`, `memory_total`, `hostname`, `user`, `home`, `temp`, `timezone`, `path_style`, `nurt_version`, `features` |
| `environment_variable` | `name`, `value`, `scope` |
| `account` | `name`, `kind`, `id` |
| `log_event` | `time`, `source`, `level`, `message` |

`open_files` produces `process` records of the processes that hold the path.
Environment values are never masked. The raw attributes of the system are
kept in `os_raw` on the kinds that have the field and are outside the
stability guarantee of the record.

## Scope of environment variables

The parameter `scope` of `environment` selects where the variables are read:

| Value | Source |
|---|---|
| `process` | environment of the process that runs the program; default |
| `user` | persistent variables of the account; Windows only |
| `machine` | persistent variables of the computer; Windows only |

`scope` of `environment` and `scope` of `services` have different values and
different meaning. For `services` it selects the service manager instance
(`system` or `user`), see the group `administration`.

The environment is read through the `env:` provider of the file commands or
through `environment`. The provider addresses one variable as a path and
returns text. `environment` returns records that can be filtered and sorted,
and it reads the persistent scopes.

## Platform differences

| Command | Linux | macOS | Windows |
|---|---|---|---|
| `processes` | process table of the kernel | process table of the kernel | process snapshot of the system |
| `ports` | socket tables of the kernel | socket tables of the system | IP Helper tables |
| `disks` | mount table | mount table | volumes with drive letters |
| `host` | kernel version in `kernel` | kernel version in `kernel` | `os_version` is the build number |
| `environment` | `process` scope | `process` scope | `process`, `user` and `machine` scopes |
| `accounts` | account and group databases | local directory | local accounts and groups; `id` is a SID |
| `logs` | systemd journal read by a native reader | unified log | Event Log |

Accounts report `uid` as UID on Linux and macOS and as SID on Windows.

Reading data that belongs to other accounts can require a privilege.
Processes, sockets and open files of other accounts are reported with the
fields that the system discloses; optional fields such as `pid`, `process`,
`cwd` or `executable` are absent when it hides them. A command that cannot
read its source at all fails with `E_PRIVILEGE`; `fix` names the required
privilege. A source read partially is reported as the warning
`permission_denied_skipped`, which does not interrupt the program.

### The command logs

`logs` returns events from the log of the operating system without starting a
program. The newest events come first. `level` is a threshold: the events of
that importance and the more important ones are returned.

| Parameter | Linux | macOS | Windows |
|---|---|---|---|
| `source` | program that logged the event | process | provider |
| `unit` | systemd unit | subsystem | channel; default `System` and `Application` |
| `since`, `until` | time of the event | time of the event | `TimeCreated` |

Linux reads the journal files of systemd with a reader of the file format of
Danaco Nurt. On Linux without a journal the command fails with `E_NOT_FOUND`.
On Windows the `Security` channel requires a privilege and fails with
`E_PRIVILEGE` without it. Each event carries a `cursor` field that marks its position in the log.

## External programs

| Command | Marker | Condition |
|---|---|---|
| `host` | [U] | `versions: true` runs every located program once with `--version` |

No other command of the group starts a program. A [U] marker names a program
of the user that the command starts only on request.

## Examples

Show the five busiest processes named `node`:

```nurt example
processes name: "node" |> sort cpu order: desc |> first 5
```

List the listening port 8080 and the process that holds it:

```nurt example
ports state: listen |> filter port == 8080
```

List the fixed disks with less than 10 GB free:

```nurt example
disks type: fixed |> filter free < 10gb |> select mount free
```

Read the warnings of one service from the system log:

```nurt example
logs unit: "nginx.service" level: warning limit: 20
```

List the processes that hold a file open:

```nurt example
open_files "/home/user/app/data.db"
```

Report the versions of programs found in the search path:

```nurt example
host programs: ["git", "python3"] versions: true
```

Wait for two seconds:

```nurt example
sleep 2s
```

## Related

[Group administration](administration.md) manages the services that `processes` and `ports`
report. [Group program](program.md) describes installed programs. The page of
the command `sleep` is [sleep](../commands/sleep.md).

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
