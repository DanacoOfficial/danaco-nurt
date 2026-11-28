---
command: host
group: system
summary: Describe the operating system, hardware, current account and home and temporary directories.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/host.md -->
<!-- Responsibility:   Documents host. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# host

## Synopsis

```text
host [name: value]...
```

## Description

`host` returns one `host` record that describes the device, the operating
system and the account running the console. Paths in `home`, `temp`,
`programs.path` and `programs.all_paths` always use `/` as the separator; on
Windows they start with the drive letter, for example `C:/Users/user`.

| Field | Linux | macOS | Windows |
|---|---|---|---|
| `os_version` | `VERSION_ID` of `/etc/os-release` | product version | version of the system |
| `kernel` | kernel release | kernel release | kernel version |
| `memory_total`, `memory_free` | `MemTotal`, `MemAvailable` of `/proc/meminfo` | host memory statistics | memory status |
| `uptime`, `boot_time` | `/proc/uptime`, `btime` of `/proc/stat` | boot time | time since start |
| `virtualization` | container markers, `/proc/1/cgroup`, firmware vendor | hypervisor flag | not reported |
| `uid`, `groups` | user and group identifiers of the process | as Linux | security identifier, local groups |
| `locale` | `LC_ALL`, `LC_MESSAGES`, `LANG` | as Linux | user locale |

`features` lists the capabilities available to the console: `journal`,
`service`, `reflink`, `hardlink`, `exchange`, `xattr`, `acl` and
`case_sensitive`.

`service` is the state of the background service of the instance. It is
`running` when the heartbeat file `service.hb` in the state directory is no
older than `space.heartbeat_max_age_ms`, the process named in it exists, and
the service answers `ping` on its control channel within 50 ms. It is
`stopped` when the service is installed (a systemd unit
`nurt-service.service`, a launchd property list `nurt.service.plist`, or the
Windows service `NurtService`) but not alive, and `not_installed` otherwise.

`programs` resolves each name in `PATH` of the console process, with the
extensions of `PATHEXT` on Windows, and returns one `program_location` per
name in the order given. `path` is the first match and `all_paths` every
match in order of resolution.

With `versions: true`, `version` is read from the metadata of the program
file: the version resource of a PE file, the `Info.plist` of an application
bundle or the one embedded in a Mach-O file, or the `.note.package` note of
an ELF file. A program without such metadata has an empty `version`. No
program is started.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `programs` | `list<text>` | `[]` | Program names to locate in PATH. |
| `versions` | `bool` | `false` | Run each located program with --version to read its version. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `host` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |
| External use | `user_program`; used when `versions` is given; Runs each located program once with --version. |

## Examples

Describe the device:

```nurt example
host
```

```text expected
os: linux
architecture: x86_64
```

Locate two programs in `PATH`:

```nurt example
host programs: ["git", "cargo"]
```

Locate a program and read its version from the file metadata:

```nurt example
host programs: ["git"] versions: true |> select programs
```

Check whether the background service runs:

```nurt example
host |> select service
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_TYPE` | `programs` is not a list of names, or `versions` is not a boolean. | Pass a list of text values, for example `["git"]`. |

## Related

`environment`, `accounts`, `processes`, `disks`, `program.find`, `service`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
