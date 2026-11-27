---
group: schedule
summary: Schedules of pipelines.
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/groups/schedule.md -->
<!-- Responsibility:   Documents Group schedule. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Group schedule

## Scope

The group creates and manages schedules: programs that run at set times
without a caller. One syntax replaces cron, systemd timers, launchd jobs and
the Windows Task Scheduler. The action of a schedule is a program in the
syntax of Danaco Nurt, given as a pipeline in the parameter `action`.

Schedules run in the background service of the instance (`nurt-service`). A
run is an ordinary operation: it has an entry in the operation journal and
the effects of its program are reported like those of a program run by a
caller. The group only defines and queries schedules; the service executes
them.

| Task | Command | Group |
|---|---|---|
| Start or stop the background service | `nurt service` | command line |
| Run a program once, now | `run` | core |
| Undo the effect of a scheduled run | `undo` | core |
| List the operations of the journal | `history` | core |

## Commands

| Command | Category | Input and output | Reversibility | Page |
|---|---|---|---|---|
| `schedule.create` | change | none to `schedule` | reversible | [schedule.create](../commands/schedule.create.md) |
| `schedule.list` | source | none to `schedule` | read | [schedule.list](../commands/schedule.list.md) |
| `schedule.remove` | change | none to `effect` | reversible | [schedule.remove](../commands/schedule.remove.md) |
| `schedule.pause` | change | none to `effect` | reversible | [schedule.pause](../commands/schedule.pause.md) |
| `schedule.resume` | change | none to `effect` | reversible | [schedule.resume](../commands/schedule.resume.md) |
| `schedule.run` | run | none to `exec` or `job` | reversible if preserved | [schedule.run](../commands/schedule.run.md) |
| `schedule.history` | source | none to `schedule_run` | read | [schedule.history](../commands/schedule.history.md) |

## Records

| Record | Required fields |
|---|---|
| `schedule` | `id`, `trigger`, `next_run`, `active`, `action` |
| `schedule_run` | `schedule_id`, `started`, `exit_state`, `trigger_time` |
| `effect` | `status`, `kind` |
| `exec` | `command`, `program`, `cwd`, `pid`, `exit_code`, `started`, `elapsed` |

The field `trigger` is an object with `kind` (`cron`, `every` or `at`),
`expression` (the trigger as written) and `timezone`. The field `action` holds
the canonical text of the program. The values of `exit_state` are `ok`,
`failed`, `timed_out`, `skipped_overlap`, `missed` and `cancelled`; the field
`operation` of a run is the number of its journal operation.

## Triggers and time

`schedule.create` takes the label as the first argument and exactly one
trigger:

| Parameter | Trigger |
|---|---|
| `cron` | five-field cron expression or `@daily` |
| `every` | interval from `30s` to 366 days |
| `at` | one point in time |

The first run of an interval is one interval after creation. A later run is
one interval after the previous planned time; when that time has passed, it is
one interval after the current time. A planned time is always later than the
reference time.

`timezone` defaults to `local`, the zone of the computer at the moment of
creation stored as an IANA name. A later change of the zone of the system
does not move the schedule. A time that does not exist in a transition to
summer time runs at the first existing moment after the transition. A time
that occurs twice in a transition to winter time runs once, at the first
occurrence.

## Missed runs and overlap

The background service runs only while the computer and the service run. A
planned time that passed while they did not is handled by `on_missed`:

| Value | Behavior |
|---|---|
| `skip` | The run is skipped when the delay exceeds `misfire` (default `2m`); the next run is planned from the current time. |
| `run_once` | Exactly one run is made, regardless of the number of missed times. |

`overlap` decides what happens when a planned time arrives while the previous
run still runs:

| Value | Behavior |
|---|---|
| `skip` | The run is skipped and counted as skipped; default. |
| `queue` | One run is kept for later; further times count as skipped. |
| `parallel` | The run starts at once, alongside the previous one. |

`max_runs` ends the schedule after that many runs, and `expires` ends it at a
time or after a duration. A finished schedule has the state `completed` or
`expired`.

`timeout` (default `1h`) limits the whole program of the action and is the
default `timeout` of its `run` stages that do not set one. Without this rule
the default limit of `run` would stop an action earlier than the schedule
allows.

`schedule.run` starts the action at once, outside the planned times. It does
not change `next_run` and does not count against `max_runs`.

## Platform differences

The schedule is the same on all systems. The service differs:

| Aspect | Linux | macOS | Windows |
|---|---|---|---|
| Service manager of the background service | systemd | launchd | service control manager |
| Service in user mode | runs for the account; it keeps running after logout only when linger is enabled for the account | runs for the account | runs for the account |
| Service in system mode | runs for the computer | runs for the computer | runs for the computer |

The installation mode of the service is the setting `service.mode`, `user` by
default and `system` for servers. Without a running service a schedule is
stored but does not run: `schedule.create` returns the record with
`active: false` and `reason: service_not_running`; the failure is reported
and not hidden.

## External programs

No command of the group starts a program. The group has no [X], [P] or [U]
marker. On macOS the service is registered with launchd by `nurt service`,
outside these commands.

## Examples

Create a schedule that runs a backup program every night at 02:00:

```nurt example
schedule.create "backup" cron: "0 2 * * *" action: (run ["backup"] cwd: "/home/user")
```

Check an address every five minutes and keep the run limited to one minute:

```nurt example
schedule.create "health" every: 5m timeout: 1m action: (http.request "https://example.com/health" |> assert all: status < 400)
```

List the active schedules with their next run:

```nurt example
schedule.list state: active |> select id label next_run
```

Create a schedule, pause it and resume it:

```nurt example
schedule.create "report" every: 1h action: (run ["report"] cwd: "/home/user")
schedule.pause "s1"
schedule.resume "s1"
```

Create a schedule and run its action once at once:

```nurt example
schedule.create "pause" every: 10m action: (sleep 2s)
schedule.run "s1"
```

Show the failed runs of all schedules:

```nurt example
schedule.history limit: 10 |> filter exit_state == "failed"
```

## Related

[Group network](network.md) provides the requests that schedules commonly run.
[Group system](system.md) provides `sleep`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
