---
command: schedule.history
group: schedule
summary: List past runs of schedules with their result and journal operation.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# schedule.history

## Synopsis

```text
schedule.history [<id>] [name: value]...
```

## Description

`schedule.history` returns the runs recorded by the background service, the
newest start first, at most `limit` records. With `id` it returns only the
runs of that schedule; the runs of a removed schedule remain available.

Each record describes one run or one run time that did not run. `exit_state`
holds the outcome: `ok`, `failed` (a non-zero exit code, a refused or failed
run), `timed_out`, `cancelled` (a stopped run), `missed` (a run time skipped
because it was late or no client provided the action) and `skipped_overlap`
(a run time reached while a run was in progress). `trigger_time` is the run
time, or the start for manual runs. `message` names the run, its trigger and
its outcome, for example `#4 schedule ok` or
`#5 schedule skipped (misfire)`. `operation` is the journal operation of the
run.

The service keeps `schedule.history_keep` runs per schedule (default 20).
Manual runs made without the service are not recorded.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `id` | `text` | none | Only runs of this schedule. |
| `limit` | `int` | `20` | Maximum number of runs, newest first; at least 1. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `schedule_run` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

List the last twenty runs of all schedules:

```nurt example
schedule.history
```

List the last five runs of one schedule:

```nurt example
schedule.history "s1" limit: 5
```

List the runs that did not succeed:

```nurt example
schedule.history limit: 100 |> filter exit_state != "ok" |> select schedule_id, started, message
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | No schedule and no recorded run has the identifier `id`. | List the schedules with `schedule.list`. |
| `E_INVALID_VALUE` | `limit` is below 1 or `id` is not a schedule identifier. | Give a positive `limit` and an identifier such as `s3`. |
| `E_FORMAT` | `<state>/schedules/state.json` is damaged. | Restart the background service; it rewrites the file. |
| `E_IO` | The schedule store cannot be read. | Check the permissions of the state directory. |

## Related

`schedule.list` shows the last result of each schedule; `schedule.run` runs
a schedule outside its run times.
