---
command: processes
group: system
summary: List running processes.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# processes

## Synopsis

```text
processes [<name>] [name: value]...
```

## Description

`processes` lists the processes of the host as `process` records, ordered by
`pid`. `name`, `user` and `pid` select processes; all given conditions must
hold. A text `name` matches the whole process name, and on Windows also the
name followed by `.exe`; a regular expression matches any part of the name.
`user` matches the account name or the account identifier (`uid`).

With `tree: true` the selected processes are the roots of a tree. The output
holds each root followed by all its descendants in depth-first order,
children ordered by `pid`, with the fields `depth` (0 for a root) and
`children`. A selected process that descends from another selected process
appears once, under its ancestor. Without conditions the roots are the
processes whose parent is not in the table. Parent links that form a cycle
are cut at the repeated process and reported as a warning.

`cpu` is the processor use in percent of one core. When the rest of the
pipeline names the fields it uses and `cpu` is among them, the command reads
the processor time twice, 200 ms apart, and reports the use in that interval.
Otherwise `cpu` is the average over the lifetime of the process, computed
from one reading without waiting. A process that ends between the two
readings is left out; a process that starts between them has no `cpu`.

`memory` is the resident memory. `uid` is the effective user identifier on
Linux and macOS and the security identifier of the process token on Windows.
`cwd` is read only with `cwd: true`. Without input conditions the command
reads the table lazily, so a following limit stops the reading early.

| Data | Linux | macOS | Windows |
|---|---|---|---|
| process table | `/proc/<pid>/stat`, `status` | `proc_listpids`, `proc_pidinfo` | Toolhelp snapshot |
| `command` | `/proc/<pid>/cmdline` | `sysctl(KERN_PROCARGS2)` | process environment block |
| `executable` | `/proc/<pid>/exe` | `proc_pidpath` | `QueryFullProcessImageNameW` |
| `cwd` | `/proc/<pid>/cwd` | `PROC_PIDVNODEPATHINFO` | process environment block |
| `state` | state letter of `stat` | `pbi_status` | always `running` |

Fields of processes of other accounts that the account may not read, such as
`executable`, `command` and `cwd`, stay empty. The report carries one
`source_unavailable` note with the number of such processes. On Linux a
kernel thread has an empty `command` and its name in square brackets, for
example `[kthreadd]`. A state without a counterpart is `other`, and
`os_raw.state` holds the code of the system.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text \| regex` | none | Process name or pattern. Positional. |
| `user` | `text` | none | Only processes of this account. |
| `pid` | `int` | none | Only the process with this identifier. |
| `cwd` | `bool` | `false` | Include the working directory of each process. |
| `tree` | `bool` | `false` | Return processes as a tree with child lists and depth. |

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

List the processes named `nginx` with their child processes:

```nurt example
processes name: "nginx" tree: true
```

List the processes of one account with their working directories:

```nurt example
processes user: "www-data" cwd: true
```

Show one process by its identifier:

```nurt example
processes pid: 1
```

Select processes by a pattern of the name:

```nurt example
processes /^python[0-9.]*$/
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | `name` is not a valid regular expression, or `pid` is negative. | Correct the pattern or the identifier. |
| `E_PRIVILEGE` | The process table is not readable by the account. | Run with the privilege named in `required`. |
| `E_IO` | The process table cannot be read, for example `/proc` is not mounted. | Mount `proc` at `/proc`. |
| `E_CANCELLED` | The execution was cancelled during the measurement of `cpu`. | Run the program again. |

## Related

`ports`, `open_files`, `suspend`, `resume`, `stop`.
