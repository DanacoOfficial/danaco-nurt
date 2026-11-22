---
command: retry
group: core
summary: Run the command of a job again with the same working directory and environment.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# retry

## Synopsis

```text
job |> retry [name: value]...
```

## Description

Runs the command of every input job again under the rules of `run`: the same
program and arguments, working directory, environment, standard input,
limits, change detection and output selection. The new job records the
repeated job in `retry_of`. A job of `test.*` or `analyze.*` is repeated as
such, so that its final record is produced again.

The environment of a job is kept outside the job record, in a file of the
state directory readable only by the account of the engine. Neither job
records nor events contain environment values.

Only finished jobs can be repeated. When the new program runs longer than
`wait`, it continues in the background; the result is then a `job` record and
the stages after `retry` are skipped (`post_stages_deferred`).

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `timeout` | `duration` | the time limit of the original job | Time limit of the program. |
| `wait` | `duration \| enum(forever)` | the default wait of run | Time to wait for the program before it continues as a job. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `job` | `exec`, `job` |

Input `job` records. Produces an `exec` record for each program that ended
within `wait` and a `job` record for each program that continues in the
background.

## Effect

| Property | Value |
|---|---|
| Category | `run` |
| Effect | `process` |
| Reversibility | `reversible_if_preserved` |
| Implementation | `user_program` |
| Reversibility conditions | As for run: changes of watched files are reversible when kept; other effects are irreversible. |

## Examples

Repeat job `j7`.

```nurt example
jobs "j7" |> retry
```

```text expected
exit_code: 0
```

Repeat job `j7` with a longer time limit and wait until it ends.

```nurt example
jobs "j7" |> retry timeout: 30m wait: forever
```

Repeat every failed job in the background.

```nurt example
jobs state: failed |> retry wait: 0s
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_JOB_STATE` | The job still runs. | Stop it first: `jobs "j7" \|> stop`. |
| `E_UNKNOWN_JOB` | No job has the identifier. | Use one of the identifiers in `candidates`. |
| `E_PROGRAM_NOT_FOUND` | The program cannot be started again. | Check the program and the working directory. |
| `E_SPAWN` | The program cannot be started again. | Check the program and the working directory. |

## Related

`run`, `jobs`, `stop`.
