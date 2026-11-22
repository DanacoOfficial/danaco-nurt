---
command: schedule.run
group: schedule
summary: Run the action of a schedule now, outside its times and run limit.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# schedule.run

## Synopsis

```text
schedule.run <id>
```

## Description

`schedule.run` runs the action of a schedule once and returns the output
records of the action, for example the `exec` record of a `run` stage. The
run does not change the next run time and does not count against `max_runs`.
Paused, completed and expired schedules can be run as well.

With a running background service, the service executes the run as an
operation of the session `schedule:<id>`, records it in the history with
`trigger: manual` and returns when it ends. A run of the same schedule that
is still in progress, scheduled or manual, makes the command fail with
`E_JOB_STATE`; the lock is shared by the service and every console process.

Without the service, the calling process runs an action of kind `pipeline`
itself, in the current session; such a run is not recorded in the history.
Actions of other kinds run only in the service.

A definition whose action no longer matches its integrity hash is not run.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `id` | `text` | required | Schedule to run. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `exec`, `job` |

## Effect

| Property | Value |
|---|---|
| Category | `run` |
| Effect | `process` |
| Reversibility | `reversible_if_preserved` |
| Implementation | `builtin` |
| Reversibility conditions | File changes are reversible as for run; effects outside the file system are irreversible. |

## Examples

Run a schedule now:

```nurt example
schedule.run "s1"
```

Run a schedule and keep its exit code and standard output:

```nurt example
schedule.run "s1" |> select exit_code, stdout
```

Create a schedule and test its action at once:

```nurt example
schedule.create "report" cron: "0 7 * * mon" action: (run ["./weekly-report.sh"] cwd: "/srv/reports")
schedule.run "s1"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `id` is missing. | Give the identifier of the schedule. |
| `E_NOT_FOUND` | No schedule has the identifier `id`. | List the schedules with `schedule.list`. |
| `E_JOB_STATE` | A run of the schedule is in progress; `actual` holds its identifier, for example `s1#4`. | Wait until the run ends, or follow it with `schedule.history`. |
| `E_STAGE_FAILED` | The action cannot run here: its kind runs only in the service, or its definition was changed outside the schedule commands. | Start the background service, or remove the schedule and create it again. |
| `E_INVALID_VALUE` | `id` is not a schedule identifier. | Give an identifier such as `s3`. |

Errors of the action itself, such as `E_TIMEOUT`, are returned as reported by
its stages.

## Related

`schedule.history` lists the runs made by the service; `schedule.create`
with `run_now: true` runs a new schedule at creation.
