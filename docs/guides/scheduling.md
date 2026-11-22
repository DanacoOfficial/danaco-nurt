---
name: scheduling
description: Running pipelines on a schedule with cron expressions, intervals and times.
commands: [schedule.create, schedule.list, schedule.run, schedule.pause, schedule.resume, schedule.remove, schedule.history, run]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# scheduling

## Creating a schedule

`schedule.create` takes a label and an action, which is a pipeline, and one of the times `cron`, `every` or `at`. `timezone` selects the zone in which the times are read, `max_runs` and `expires` end the schedule, and `enabled: false` creates it paused. `run_now: true` starts the first run at once.

```nurt
schedule.create "backup" cron: "0 3 * * *" action: (run ["backup"] cwd: "/srv")
schedule.create "health" every: 5m action: (http.request "https://example.com/health" |> assert count: 1)
```

## Missed runs and overlap

`on_missed` decides what happens to runs that were due while the service was stopped: `skip` drops them and `run_once` runs the action once at start. `overlap` decides what happens when a run starts while the previous one still runs: `skip` drops the new run, `queue` waits for the previous one and `parallel` starts both.

```nurt
schedule.create "sync" every: 1h action: (run ["sync"] cwd: "/srv") on_missed: run_once overlap: skip
```

## The service

Schedules run only while `nurt service` is running. When it is not, a schedule is stored and `schedule.list` reports `active: false` for it. The next run and the state of each schedule are fields of the `schedule` record.

```nurt
schedule.list state: active
```

## Controlling schedules

`schedule.pause` and `schedule.resume` stop and restart a schedule without removing it, `schedule.remove` deletes it and `schedule.run` runs the action now, outside its times and run limit. The first argument is an `id`, or `all: true` for every schedule.

```nurt
schedule.pause "backup"
schedule.run "backup"
```

## History

`schedule.history` lists past runs with their result and the journal operation of the changes they made. A change made by a scheduled run can be undone like any other change.

```nurt
schedule.history "backup" limit: 10
```
