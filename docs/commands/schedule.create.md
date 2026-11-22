---
command: schedule.create
group: schedule
summary: Create a schedule that runs a pipeline by cron expression, interval or time.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# schedule.create

## Synopsis

```text
schedule.create <label> action: <pipeline> (cron: <text> | every: <duration> | at: <instant>) [name: value]...
```

## Description

`schedule.create` saves a schedule and returns its `schedule` record. The
schedule runs its action at the run times of exactly one trigger:

- `cron` takes five fields: minute (0-59), hour (0-23), day of month (1-31),
  month (1-12 or `jan`-`dec`) and day of week (0-7 or `sun`-`sat`, where 0 and
  7 are Sunday). A field holds `*`, numbers, names, ranges `a-b`, lists `a,b`
  and steps: `*/n` from the lower bound, `a-b/n` within the range and `a/n`
  from `a` to the end of the range. Names are not case sensitive. When neither
  the day of month nor the day of week starts with `*`, a day matches when
  either field matches; otherwise both must match. The shortcuts `@yearly`,
  `@annually`, `@monthly`, `@weekly`, `@daily` and `@hourly` are accepted.
  An expression without a run time within five years is rejected.
- `every` runs at a fixed interval. The first run time is the creation time
  plus the interval. The interval lies between `schedule.min_interval`
  (default `30s`) and `schedule.max_interval` (default `366d`).
- `at` runs once at the given time, which must lie in the future.

Every run time is strictly later than the time it is computed from: a
schedule created at 14:40:00 with `cron: "*/10 * * * *"` first runs at 14:50.

`timezone` applies to the fields of a cron expression and to the times shown
by `schedule.list`. The value `local` is replaced by the IANA name of the
time zone of the device at creation, so a later change of the time zone of
the device does not move the run times. A wall-clock time skipped by the
change to daylight saving time runs at the same distance after the change
(02:30 becomes 03:30); a wall-clock time repeated by the change back runs
once, at its first occurrence.

The action is a pipeline of the language. It is bound and checked when the
schedule is created and saved together with an integrity hash. Each run
executes the pipeline as an operation of the session `schedule:<id>` with the
base directory and the project directories of the creating call, so every run
has its own entry in the operation journal. The environment of the programs
of the action is the environment of the background service with
`run.env_defaults`; variables of the calling session are passed explicitly
with the `env` parameter of `run`. `timeout` limits the whole run and is the
time limit of the `run` stages of the action that set none. A definition
whose action no longer matches its integrity hash is not run: the run is
recorded with `state: rejected` and `reason: definition_changed`.

Schedules run in the background service of the account. When the service
does not run, the schedule is saved and the record carries `active: false`
and `reason: service_not_running`; the service applies the schedule when it
starts. With `run_now: true` and a running service the action also runs once
at creation, outside the run times and the run limit.

A run time reached later than `misfire` after it is not made up one by one:
`on_missed: skip` records one skipped run (`reason: misfire`) and
`on_missed: run_once` makes exactly one run (`trigger: missed`); in both cases
the next run time counts from the present. A single time (`at`) skipped this
way makes the schedule `expired`. A run time reached while a run of the
schedule executes is skipped (`overlap: skip`), waits in a queue of one place
(`overlap: queue`) or runs as well (`overlap: parallel`, within
`schedule.max_parallel`).

The identifiers are `s1`, `s2` and so on. The number of schedules that are
not completed or expired, paused ones included, is limited by
`schedule.max_active` (default 50).

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `label` | `text` | required | Name of the schedule. |
| `action` | `pipeline` | required | Pipeline to run. |
| `cron` | `text` | one of `trigger` | Five-field cron expression or a shortcut such as @daily. |
| `every` | `duration` | one of `trigger` | Interval between runs; at least the configured minimum. |
| `at` | `instant` | one of `trigger` | Single run at this time. |
| `timezone` | `text` | `"local"` | Time zone of the cron expression and times. |
| `on_missed` | `enum(skip\|run_once)` | `skip` | Behaviour for runs missed while the service was not running. |
| `misfire` | `duration` | `2m` | Maximum delay after which a missed run is not made up. |
| `overlap` | `enum(skip\|queue\|parallel)` | `skip` | Behaviour when a run is due while the previous one is still running. |
| `timeout` | `duration` | `1h` | Time limit of each run. |
| `max_runs` | `int` | none | Number of runs after which the schedule completes. |
| `expires` | `instant \| duration` | none | Time, or time from now, after which the schedule expires. |
| `run_now` | `bool` | `false` | Also run once immediately. |
| `enabled` | `bool` | `true` | Create the schedule active rather than paused. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `schedule` |

The record holds `id`, `label`, `trigger` (`kind`, `expression`, `timezone`),
`next_run`, `state`, `active`, `reason`, `action`, `action_kind`, `runs`,
`max_runs`, `expires`, `on_missed`, `misfire`, `overlap`, `timeout`,
`created` and `session`.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

The definition file is written through the operation journal. The run state
(`<state>/schedules/state.json`: next run times, counters and history) is
written only by the background service and is not part of the journal.

## Examples

Run a backup every day at 03:00 in the time zone of the device:

```nurt example
schedule.create "backup" cron: "0 3 * * *" action: (run ["tar", "-czf", "/srv/backup.tgz", "data"] cwd: "/srv")
```

Check a directory every ten minutes and keep at most one waiting run:

```nurt example
schedule.create "inbox" every: 10m overlap: queue action: (run ["./process-inbox.sh"] cwd: "/srv/inbox")
```

Run once at a given time and make up the run if the device sleeps at that
time:

```nurt example
schedule.create "reminder" at: "2026-12-24T18:00:00+01:00" on_missed: run_once action: (run ["notify-send", "Stand-up"] cwd: "/tmp")
```

Run on weekdays between 09:00 and 17:45 every quarter of an hour, for twenty
runs at most:

```nurt example
schedule.create "probe" cron: "*/15 9-17 * * mon-fri" timezone: "Europe/Warsaw" max_runs: 20 action: (run ["./probe"] cwd: "/opt/probe")
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `label` is missing, or none of `cron`, `every` and `at` is given. | Give the label and exactly one trigger. |
| `E_PARAM_CONFLICT` | More than one of `cron`, `every` and `at` is given. `expected` lists the parameters of the group. | Keep one trigger. |
| `E_INVALID_VALUE` | The cron expression is invalid or has no run within five years, the interval lies outside the configured bounds, `at` or `expires` has passed, the time zone is unknown, `timeout` exceeds `schedule.max_timeout`, or `max_runs` is below 1. | Correct the value named in the message; `fix` holds a valid form where one exists. |
| `E_UNKNOWN_VALUE` | `on_missed` or `overlap` has an unknown value. | Use one of the listed values. |
| `E_TYPE` | A parameter has the wrong type, for example a number for `every`. | Give a value of the listed type, such as `10m`. |
| `E_LIMIT` | `schedule.max_active` schedules exist. | Remove schedules with `schedule.remove` or raise the limit. |
| `E_IO` | The schedule store cannot be read or written. | Check the permissions of the state directory. |

## Related

`schedule.list` shows schedules and their next run times, `schedule.run` runs
an action outside its run times, `schedule.pause` and `schedule.resume`
suspend and resume schedules, `schedule.remove` removes them and
`schedule.history` lists past runs.
