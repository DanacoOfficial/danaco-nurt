---
command: logs
group: system
summary: Read events from the system log: systemd journal, Windows Event Log or macOS unified log.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# logs

## Synopsis

```text
logs [source: <text>] [unit: <text>] [level: critical|error|warning|info|debug] [since: <instant>] [until: <instant>] [message: <regex>] [limit: <int>]
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
when older events remain, the result ends with the notice `log_limit_reached`
and the `cursor` of the last event marks where the next page starts.

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

On Windows the command reads the Event Log. `unit` names the channel, such as
`System`, `Application` or `Microsoft-Windows-TaskScheduler/Operational`, and
`source` names the provider. Channel and provider names are compared without
regard to case. Event levels map as follows: 1 is `critical`, 2 `error`,
3 `warning`, 0 and 4 `info`, 5 `debug`.

### macOS

On macOS the command reads the unified log. `source` names the subsystem,
such as `com.apple.xpc`, and `unit` names the category.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `source` | `text` | none | Event source: identifier, provider or subsystem. |
| `unit` | `text` | none | Service unit, event channel or category. |
| `level` | `enum(critical\|error\|warning\|info\|debug)` | `info` | Lowest level of events to return. |
| `since` | `instant` | none | Only events at or after this time. |
| `until` | `instant` | none | Only events before this time. |
| `message` | `regex` | none | Only events whose message matches this pattern. |
| `limit` | `int` | `100` | Maximum number of events, newest first; at most the configuration value `logs.max_limit`. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `logs` | none | `log_event` |

The `log_event` record holds `time`, `source`, `level` and `message`, and where
the log provides them `id`, `host`, `unit`, `pid`, `boot_id`, `cursor` and
`fields`. On Linux `id` is `MESSAGE_ID` and `fields` holds the other fields of
the entry except `MESSAGE`, `MESSAGE_ID`, `_HOSTNAME`, `_PID` and `_BOOT_ID`.

Values longer than `logs.max_message_bytes` (message) or `logs.max_field_bytes`
(other fields), and fields beyond `logs.max_fields`, are cut; the event is then
marked as truncated. A message that is not valid UTF-8 is decoded with
replacement characters.

| Notice | Meaning |
|---|---|
| `log_limit_reached` | More events match than `limit` allows. |
| `log_unknown_name` | No event has the given `source` or `unit`; `detail` lists similar names. |
| `log_file_skipped` | A log file was skipped; `detail` is `truncated`, `corrupt`, `unsupported_format` or `io`. |
| `permission_denied_skipped` | Some log files could not be read for lack of permission; `count` gives their number. |
| `log_cursor_approximate` | The log file of the cursor no longer exists; paging continues from the time of the cursor. |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read`: nothing is changed, so there is nothing to undo |
| External programs | none |
| Waits | no |

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

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | The system has no log that `logs` reads, such as a Linux system without a systemd journal. | Read text log files such as `/var/log/syslog` with `read`. |
| `E_PERMISSION` | No log file is readable by the account. | On Linux add the account to the group `systemd-journal` or `adm`, or run as root. |
| `E_FORMAT` | No log file has a readable format; `files` lists the files with the class of their defect. | Check the files; files written by a newer systemd need a newer release of Danaco Nurt. |
| `E_INVALID_VALUE` | A parameter has a value of the wrong form, such as a cursor of another system. | Use a cursor returned by `logs` on the same system. |
| `E_REGEX` | The pattern of `message` is not a valid regular expression. | Correct the pattern. |
| `E_LIMIT` | `limit` exceeds `logs.max_limit`. | Lower `limit` or page with the cursor of the last event. |
| `E_CANCELLED` | The execution was cancelled between two events. | Run the program again. |
| `E_TIMEOUT` | The time limit of the program elapsed while the log was read. | Narrow the query with `since`, `unit` or `source`. |

## Related

`services` lists the services whose `unit` names `logs` accepts. `filter`,
`group` and `sort` process the `log_event` records further. The `system` group
page lists the other commands of the group.
