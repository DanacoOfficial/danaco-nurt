---
command: test.failed
group: verify
summary: Run again only the tests that failed in an earlier run.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/test.failed.md -->
<!-- Responsibility:   Documents test.failed. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# test.failed

## Synopsis

```text
test.failed [<run_id>] [name: value]...
```

## Description

Selects the tests with the status `failed` or `error` of an earlier run and
runs only those. The identifiers of the tests are translated to the selectors
of the framework: files and a name pattern for bun, vitest and jest, node
identifiers for pytest, packages and `-run` for go and `--exact` names for
cargo. The `npm` scripts and custom commands cannot select single tests; the
stored command runs again and the command logs a note.

Without `run_id` the run is the newest run that has failed tests and that no
later run repeated. With `path` only runs of that project directory are
considered, and the repeat runs in it. The new run records the repeated run in
`retry_of`.

When the run has no failed tests, the command returns its `test_run` header
and no results, and starts nothing.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `run_id` | `text` | the latest run | Earlier test run. |
| `path` | `path` | none | Project directory of the earlier run. |
| `timeout` | `duration` | `30m` | Time limit of the test run. |
| `wait` | `duration` | `3s` | Time to wait before the run continues in the background as a job. |
| `failures` | `int` | `10` | Maximum number of failed tests reported in detail. |
| `message_lines` | `int` | `5` | Maximum number of message lines per failed test. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `test_run`, `test_result`, `job` |

The command takes no input. The output is the same as the output of
`test.run`: a `test_run` header followed by `test_result` records, or a `job`
record.

## Effect

| Property | Value |
|---|---|
| Category | `run` |
| Effect | `process` |
| Reversibility | `reversible_if_preserved` |
| Implementation | `user_program` |
| Reversibility conditions | Files changed by the test programs are reversible when their previous content was preserved; effects outside the file system are irreversible. |

## Examples

Run again the failures of the latest run.

```nurt example
test.failed
```

Run again the failures of a named run and show the failed results.

```nurt example
test.failed run-3 |> filter status == "failed"
```

Run again the failures of the run of another project directory.

```nurt example
test.failed path: "/work/service"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_UNKNOWN_RUN` | The run does not exist, or no run has failures. | List the runs with `test.list`. |
| `E_EXTERNAL_MISSING` | The test program is not installed. | Install it. |
| `E_OUTPUT_PARSE` | The report or the output cannot be read. | Run again with `test.run command:`. |

## Related

`test.run`, `test.show`, `test.list`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
