---
command: stop
group: core
summary: Stop jobs, processes or services.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# stop

## Synopsis

```text
… |> stop [signal: terminate|kill|interrupt|hangup] [grace: duration]
```

## Description

Stops every input job: the signal `signal` goes to the whole process group
or job object of the program, and after `grace` the remaining processes are
killed. The command returns after the job has ended and its final record,
its detected changes and its journal entry are written.

Only running and suspended jobs can be stopped. Job records are handled by
this implementation; process and service records are handled by the
implementations of the system commands, selected by the record kind of the
input.

On Windows `terminate`, `interrupt` and `hangup` request the end of the
console program group; `kill` ends the job object.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `signal` | `enum(terminate\|kill\|interrupt\|hangup)` | `terminate` | Signal sent first. |
| `grace` | `duration` | `5s` | Time to wait before the process is killed. |

## Input and output

Input `job`, `process` or `service` records. Produces one `effect` record per
input record; for a job its `job` field holds the final job record with the
state `stopped`.

## Effect

Category `run`, effect `external`, reversibility `irreversible`. Every
stopped job is reported as irreversible with the reason `process_stopped`.
Changes the stopped program made to watched files are recorded in the
journal entry of the job.

## Examples

Stop job `j7`.

```nurt example
jobs "j7" |> stop
```

```text expected
stopped  job:j7
```

Stop all running jobs at once, without a grace period.

```nurt example
jobs state: running |> stop signal: kill
```

Interrupt job `j3` and kill it after ten seconds.

```nurt example
jobs "j3" |> stop signal: interrupt grace: 10s
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_JOB_STATE` | The job has already ended or lost its supervisor. | Read the job with `jobs "j7"`. |
| `E_UNKNOWN_JOB` | No job has the identifier. | Use one of the identifiers in `candidates`. |
| `E_TIMEOUT` | The supervisor of the job did not answer. | Read the state with `jobs "j7"`. |

## Related

`jobs`, `suspend`, `resume`, `remove`.
