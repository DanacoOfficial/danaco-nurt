---
command: open_files
group: system
summary: List the processes that hold a file or directory open.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/open_files.md -->
<!-- Responsibility:   Documents open_files. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# open_files

## Synopsis

```text
open_files <path>
```

## Description

`open_files` returns one `process` record for each process that holds
`path`, ordered by `pid`. The path must be absolute and must exist; symbolic
links in it are resolved first. A file matches the processes that use that
file. A directory matches the processes that use the directory or any path
below it.

| System | Source | `os_raw.access` |
|---|---|---|
| Linux | open descriptors, working and root directories, executable and memory maps in `/proc` | `fd`, `cwd`, `root`, `exe`, `mmap` |
| macOS | open descriptors, working and root directories, executable and mapped regions reported by the kernel | `fd`, `cwd`, `root`, `exe`, `mmap` |
| Windows | Restart Manager | not set; `os_raw.kind` is `application`, `service`, `explorer`, `console`, `critical` or `unknown` |

`os_raw.files` lists the matching paths the process uses. A file deleted while
a process keeps it open matches a directory that contained it; the record
then has `os_raw.deleted: true`. A file now at the same path is another file
and does not match.

Processes whose open files cannot be read, typically processes of other
accounts, are skipped; the report carries one `permission_denied_skipped`
note with their number. Run the console as root or as an administrator to
include them. On Windows a directory is checked through the regular files
below it, up to 10 000; beyond that the report carries a
`target_count_high` note.

The file layer uses the same search to name the holders of a file in the
`E_IN_USE` error.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | File or directory. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `process` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Show the processes that hold a log file:

```nurt example
open_files "/var/log/app/current.log"
```

```text expected
name: app
user: app
```

Find the processes that keep a mounted volume busy:

```nurt example
open_files "/mnt/backup" |> select pid name user
```

Find the programs that lock a file on Windows:

```nurt example
open_files "C:/Users/user/Documents/report.xlsx"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_PATH_RELATIVE` | `path` is relative. | Give an absolute path. |
| `E_NOT_FOUND` | `path` does not exist. | Check the path with `info`. |
| `E_PRIVILEGE` | The process table or the Restart Manager refuses access. | Run the console as root or as an administrator. |
| `E_IO` | The process table cannot be read. | Check that `/proc` is mounted. |

## Related

`processes`, `ports`, `stop`, `accounts`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
