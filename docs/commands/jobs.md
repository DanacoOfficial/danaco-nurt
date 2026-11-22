---
command: jobs
group: core
summary: Read the state, output or result of background jobs.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# jobs

## Synopsis

```text
jobs [id] [state: any|running|exited|failed|stopped|timed_out] [view: status|output|result]
     [since: text] [stream: both|stdout|stderr] [head: int] [tail: int]
     [filter: regex] [context: int] [dedupe: bool] [wait: duration]
```

## Description

Every program started by `run`, `retry`, `test.*` and `analyze.*` is a job
from its start and has an identifier `j1`, `j2`, … that is never reused in
the state directory. A program that is still running after the `wait` of its
command continues in the background; `jobs` reads its state, its output and
its final result.

Without `id` the command lists all jobs: running and suspended jobs first,
then finished jobs, each group in the order of the identifiers. `state`
selects jobs in one state.

With `id` the command returns one job record, filled according to `view`:

* `status`: the state, the elapsed time and the last 5 lines of each output
  stream;
* `output`: the complete lines written after the cursor `since`, or the whole
  output without `since`; `output_cursor` of the result is the cursor for the
  next read. `stream`, `head`, `tail`, `filter`, `context` and `dedupe`
  select the lines; the cursor of a stream left out by `stream` stays where it
  was, so a later read of that stream misses no line;
* `result`: the final record of a finished job (`exit_code`, `signal`,
  `elapsed`, `changes`, `op`, output files) with the kept output selected by
  the same parameters. For a job of `test.*` or `analyze.*` the stream header
  holds the final record of the tool, a `test_run` or an `analysis`.

`wait` blocks the call until the job ends or the duration elapses, whichever
is first; the job record is then returned as it is.

A job whose supervising process ended without finishing it has the state
`lost`: its program may still run, but nothing records its output or
changes. The state is determined at every read: each supervised job holds a
lock that the operating system releases when the supervising process ends,
however it ends.

Finished jobs whose end is older than `output.spool_ttl` are removed with
their output when jobs are listed and when the supervisor starts. Running
jobs are never removed.

`remove` on `job` records deletes finished jobs: the entry of each job in
the job table and its kept output in the state directory. Job entries and
their output are internal state of the console, not user files, so the
removal adds no journal entry, cannot be undone and is not marked
irreversible. Jobs in the states `exited`, `failed`, `stopped`, `timed_out`,
`stalled`, `limit` and `lost` can be removed; for a `lost` job only the
entry and the output are deleted, and a program that still runs is not
signalled. A job in the state `running` or `suspended` is refused with
`E_JOB_STATE`; `stop` ends it first. The input is checked as a whole before
the first job is deleted. Each removed job yields one `effect` record with
`kind` and `action` `remove`, `target` `job:<id>` and the last `job` record.
A removed identifier is not reused, and a later reference to it fails with
`E_UNKNOWN_JOB`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `id` | `text` | none | Job identifier, such as `j7`. |
| `state` | `enum(any\|running\|exited\|failed\|stopped\|timed_out)` | `any` | State of the jobs to list. |
| `view` | `enum(status\|output\|result)` | `status` | State, output, or final result of the job. |
| `since` | `text` | none | Cursor of an earlier read; only newer output is returned. |
| `stream` | `enum(both\|stdout\|stderr)` | `both` | Output stream to read. |
| `head` | `int` | none | Number of first output lines returned. |
| `tail` | `int` | none | Number of last output lines returned. |
| `filter` | `regex` | none | Return only output lines that match. |
| `context` | `int` | `0` | Lines of context around lines matching `filter`. |
| `dedupe` | `bool` | `false` | Collapse repeated identical output lines. |
| `wait` | `duration` | `0s` | Time to wait for the job to finish. |

## Input and output

Source only. Produces `job` records.

## Effect

Category `source`, effect `read`, reversibility `read`. A call with `wait`
longer than zero is marked as waiting by the static check.

## Examples

List the running jobs.

```nurt example
jobs state: running
```

```text expected
j7  running  cargo build
```

Read the output of job `j7` written after an earlier read.

```nurt example
jobs "j7" view: output since: "120.4"
```

Wait up to one minute for job `j7` and return its final record.

```nurt example
jobs "j7" view: result wait: 60s
```

Remove all jobs that exited with code 0, together with their output.

```nurt example
jobs state: exited |> remove
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_UNKNOWN_JOB` | No job has the identifier. | Use one of the identifiers in `candidates`. |
| `E_JOB_STATE` | `view: result` on a job that still runs. | Add `wait:` or read `view: status`. |
| `E_JOB_STATE` | `remove` on a job that is running or suspended. | End the job with `stop`, then remove it. |
| `E_CONFIG` | The engine has no state directory. | Configure the state directory of the engine. |

## Related

`run`, `retry`, `stop`, `suspend`, `resume`, `remove`.
