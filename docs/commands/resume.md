---
command: resume
group: system
summary: Resume suspended jobs or processes.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# resume

## Synopsis

```text
job|process |> resume
```

## Description

Resumes the process tree of every input job suspended by `suspend`:
`SIGCONT` to the process group on Linux and macOS, resumption of the threads
of the job object on Windows. The job state becomes `running` again and its
time limit continues from where it stopped.

Only suspended jobs can be resumed. Process records are handled by the
implementation of the system commands.

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

Resume job `j7`.

```nurt example
jobs "j7" |> resume
```

```text expected
resumed  job:j7
```

Resume all suspended jobs.

```nurt example
jobs |> filter state == "suspended" |> resume
```

Resume job `j3` and wait for its end.

```nurt example
jobs "j3" |> resume
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_JOB_STATE` | The job is not suspended. | Suspend it first: `jobs "j7" \|> suspend`. |
| `E_UNKNOWN_JOB` | No job has the identifier. | Use one of the identifiers in `candidates`. |

## Related

`suspend`, `stop`, `jobs`.
