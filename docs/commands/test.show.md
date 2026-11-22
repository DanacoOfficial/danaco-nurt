---
command: test.show
group: verify
summary: Show the results of an earlier test run.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# test.show

## Synopsis

```text
test.show [<run_id>] [name: value]...
```

## Description

Reads a stored run and returns its `test_run` header and the `test_result`
records that match the filters. Without `status` every result is returned.
`failed` selects the results with the status `failed` and `error`; `skipped`
selects the results with the status `skipped` and `todo`.

`test` keeps the results whose identifier or name contains the text, ignoring
case. `failures` limits how many failed results keep their message, stack and
output, and `message_lines` cuts each of those texts to that number of lines.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `run_id` | `text` | the latest run | Earlier test run. |
| `test` | `text` | none | Only the test with this identifier or name. |
| `status` | `enum(failed\|passed\|skipped)` | none | Only tests with this status. |
| `failures` | `int` | `10` | Maximum number of failed tests reported in detail. |
| `message_lines` | `int` | `5` | Maximum number of message lines per failed test. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `test_result` |

The command takes no input. It produces the `test_run` header of the run and
its `test_result` records in the order of the run.

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Show the failures of the latest run with three message lines each.

```nurt example
test.show status: failed message_lines: 3
```

Show the results of one run whose identifier contains a name.

```nurt example
test.show run-2 test: "parse"
```

Show only the passed tests of the latest run.

```nurt example
test.show status: passed
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_UNKNOWN_RUN` | The run does not exist; the error lists known runs. | Use one of the listed runs or run `test.run`. |

## Related

`test.run`, `test.failed`, `test.list`.
