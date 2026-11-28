---
command: logs
group: system
summary: Read events from the system log: systemd journal, Windows Event Log or macOS unified log.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/logs.md -->
<!-- Responsibility:   Documents logs. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# logs

## Synopsis

```text
logs [name: value]...
```

## Description

`logs` reads the log of the operating system and produces one `log_event`
record per event, newest first. It starts a pipeline. Every parameter has the
meaning of a later `filter` on the field of the same name; the stores use their
indexes to find the matching events, so a narrow query reads little of the log.

`source` and `unit` take a literal name or a glob pattern (`*`, `?`, `[...]`).
A literal name that matches no event gives an empty result with the notice
`log_unknown_name`, which lists similar names of the log. `level` is the lowest
level returned: `level: warning` returns warnings, errors and critical events.
`since` is inclusive and `until` is exclusive. `message` keeps the events whose
message matches the regular expression. `limit` bounds the number of events;
when older events remain, the execution reports the notice `log_limit_reached`
with the `cursor` of the last event, after which the older events follow.

The notice `permission_denied_skipped` is an entry of `notable` in the report.
The other notices of the command are diagnostic events of the execution whose
message starts with the code of the notice. Notices are reported when the
output ends or when the rest of the pipeline stops reading it.

### Linux

On Linux the command reads the files of the systemd journal directly, without
`journalctl` and without the systemd library. It reads the persistent journal
in `/var/log/journal/<machine-id>/` and the volatile journal in
`/run/log/journal/<machine-id>/`: active files, archived files and files
renamed after an unclean shutdown (`*.journal~`). Journals of other namespaces,
containers and remote hosts are not read. Files written with XZ, LZ4 or zstd
compression, with the keyed hash and in compact mode are supported. Entries
that journald appends while the query runs are not returned.

| Parameter | Journal fields |
|---|---|
| `source` | `SYSLOG_IDENTIFIER`, else `_COMM`, else the file name of `_EXE`, else `_TRANSPORT`; `unknown` when none is set |
| `unit` | `UNIT` for messages of the service manager, `USER_UNIT` for messages of a user manager, else `_SYSTEMD_USER_UNIT`, else `_SYSTEMD_UNIT`; a literal name without a unit suffix gets `.service`, as in `journalctl -u` |
| `level` | `PRIORITY` 0-2 is `critical`, 3 `error`, 4 `warning`, 5-6 `info`, 7 `debug`; a missing priority is 6 |

A file that is damaged, truncated or written with an unknown incompatible
feature is skipped with the notice `log_file_skipped`; the events read from it
before the defect are kept. Files that the account may not read are skipped
with the notice `permission_denied_skipped`.

### Windows

On Windows the command reads the Event Log. `unit` names an event log channel,
such as `System`, `Application`, `Security` or
`Microsoft-Windows-TaskScheduler/Operational`; channel names are compared
without regard to case. Without `unit`, the channels `System` and
`Application` are read. `source` names the event provider (`Provider/@Name`).
Event levels map as follows: 1 is `critical`, 2 `error`, 3 `warning`, 5
`debug`; 0, 4 and provider levels from 16 are `info`. The message is formatted
from the metadata of the provider; without metadata it holds the payload as
`name=value` pairs and the notice `log_unformatted` gives the number of such
events.

| Condition | Result |
|---|---|
| unknown channel | `E_UNKNOWN_VALUE`, `expected` lists the nearest channel names |
| named channel denied, such as `Security` | `E_PRIVILEGE`, `required: administrator` |
| default channel denied | notice `permission_denied_skipped` |
| event log service stopped | `E_NOT_FOUND` |

### macOS

On macOS the command reads the unified log. `unit` names a subsystem, such as
`com.example.agent`, and `source` names a process. Faults are `critical`,
errors `error`, messages of type `default` and `info` are `info`, and `debug`
messages are `debug`. The system keeps debug and info messages only for a
short time. Reading the log store of the running system requires root; a
process that runs as root and is refused needs Full Disk Access for the
terminal application.

| Condition | Result |
|---|---|
| not root | `E_PRIVILEGE`, `required: root` |
| root without Full Disk Access | `E_PRIVILEGE`, `required: full_disk_access` |
| messages lost by the system | notice `log_events_lost` with the count |

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `source` | `text` | none | Event source: identifier, provider or subsystem. |
| `unit` | `text` | none | Service unit, event channel or category. |
| `level` | `enum(critical\|error\|warning\|info\|debug)` | `info` | Lowest level of events to return. |
| `since` | `instant` | none | Only events at or after this time. |
| `until` | `instant` | none | Only events before this time. |
| `message` | `regex` | none | Only events whose message matches this pattern. |
| `limit` | `int` | `100` | Maximum number of events, newest first. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `log_event` |

The `log_event` record holds `time`, `source`, `level` and `message`, and where
the log provides them `id`, `host`, `unit`, `pid`, `boot_id`, `cursor` and
`fields`. On Linux `id` is `MESSAGE_ID` and `fields` holds the other fields of
the entry except `MESSAGE`, `MESSAGE_ID`, `_HOSTNAME`, `_PID` and `_BOOT_ID`.

A message longer than 65536 bytes and a value of another field longer than
16384 bytes are cut at a character boundary, and fields beyond the first 128
are dropped; the event is then marked as truncated. A message that is not
valid UTF-8 is decoded with replacement characters.

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Show the errors of one service:

```nurt example
logs unit: "ssh" level: error
```

Find failed logins in the messages of `sshd`:

```nurt example
logs source: "sshd" message: /Failed password/ limit: 20
```

Show the five newest warnings and errors of the system:

```nurt example
logs level: warning |> first 5
```

Read the events of all timer units:

```nurt example
logs unit: "*.timer" level: debug
```

A response that leaves records out ends with the hint "more: next" and carries the field
`continuation`; the command `next` returns the kept events first and then
continues the query from the cursor of the last event, so that parts neither
overlap nor skip events that arrive later.

```text
shown 50  more: next
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | The system has no log that `logs` reads, such as a Linux system without a systemd journal. | Read text log files such as `/var/log/syslog` with `read`. |
| `E_PERMISSION` | No log file is readable by the account. | On Linux add the account to the group `systemd-journal` or `adm`, or run as root. |
| `E_PRIVILEGE` | The log needs a privilege the process does not have: a named Windows channel such as `Security`, or the macOS log store. | Run with administrator rights on Windows; run as root, and grant Full Disk Access if needed, on macOS. |
| `E_UNKNOWN_VALUE` | `unit` names a Windows channel that does not exist; `expected` lists the nearest names. | Use one of the listed channels. |
| `E_OS_ONLY` | The operating system is not Linux, Windows or macOS. | Read text log files with `read`. |
| `E_FORMAT` | No log file has a readable format; `files` lists the files with the class of their defect. | Check the files; files written by a newer systemd need a newer release of Danaco Nurt. |
| `E_INVALID_VALUE` | A parameter has a value of the wrong form, such as a cursor of another system. | Use a cursor returned by `logs` on the same system. |
| `E_REGEX` | The pattern of `message` is not a valid regular expression. | Correct the pattern. |
| `E_PATTERN_SYNTAX` | The glob pattern of `source` or `unit` is malformed. | Close the bracket or escape the special character. |
| `E_LIMIT` | `limit` is below 1 or above 100000. | Give a `limit` within the range, or narrow the query with `since` and `until`. |
| `E_CANCELLED` | The execution was cancelled between two events. | Run the program again. |
| `E_TIMEOUT` | The time limit of the program elapsed while the log was read. | Narrow the query with `since`, `unit` or `source`. |

## Related

`services` lists the services whose `unit` names `logs` accepts. `filter`,
`group` and `sort` process the `log_event` records further. The `system` group
page lists the other commands of the group.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
