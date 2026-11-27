---
command: suspend
group: system
summary: Suspend the input jobs or processes until they are resumed.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/suspend.md -->
<!-- Responsibility:   Documents suspend. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# suspend

## Synopsis

```text
job|process |> suspend
```

## Description

Suspends the whole process tree of every input job: `SIGSTOP` to the process
group on Linux and macOS, suspension of the threads of the job object on
Windows. The job state becomes `suspended`. Time spent suspended is not
counted against the time limit of the job and is reported in
`suspended_total` of the final record.

Only running jobs can be suspended. Job and process records are handled by
this implementation.

## Parameters

The command has no parameters.

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `job`, `process` | `effect` |

Input `job` or `process` records. Produces one `effect` record per input
record; its `job` field holds the job record after the change.

## Effect

| Property | Value |
|---|---|
| Category | `run` |
| Effect | `external` |
| Reversibility | `irreversible` |
| Implementation | `builtin` |

## Examples

Suspend job `j7`.

```nurt example
jobs "j7" |> suspend
```

```text expected
suspended  job:j7
```

Suspend all running jobs.

```nurt example
jobs state: running |> suspend
```

Suspend job `j3` and read its state.

```nurt example
jobs "j3" |> suspend |> select job.state
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_JOB_STATE` | The job is not running, for example already suspended. | Resume it: `jobs "j7" \|> resume`. |
| `E_UNKNOWN_JOB` | No job has the identifier. | Use one of the identifiers in `candidates`. |

## Related

`resume`, `stop`, `jobs`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
