---
command: test.list
group: verify
summary: List earlier test runs.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/test.list.md -->
<!-- Responsibility:   Documents test.list. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# test.list

## Synopsis

```text
test.list [name: value]...
```

## Description

Returns the `test_run` headers of the stored runs, newest first. The history
keeps the number of runs set by `verify.history_max` (default `50`); older runs
are removed when a new run is stored. A stored run that cannot be read is left
out of the list and reported in the notes of the result as
`source_unavailable`, with the path of its file and the advice to remove the
file; the other runs are listed. `test.show` of that run still fails with the
error of the unreadable entry.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `limit` | `int` | `20` | Maximum number of runs, newest first. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `test_run` |

The command takes no input. It produces `test_run` records.

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

List the five newest runs.

```nurt example
test.list limit: 5
```

List the runs that failed.

```nurt example
test.list |> filter status == "failed"
```

Show the identifier and the fingerprint of the latest run.

```nurt example
test.list limit: 1 |> select id fingerprint
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | `limit` is negative or too large. | Give a count between 0 and 4294967295. |

## Related

`test.run`, `test.show`, `test.failed`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
