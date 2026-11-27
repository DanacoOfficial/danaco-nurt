---
command: first
group: core
summary: Keep the first records.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/first.md -->
<!-- Responsibility:   Documents first. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# first

## Synopsis

```text
any |> first <count>
```

## Description

`first` passes on the first `count` input records and drops the rest. The
stage reads no more than `count` records from its input, so the source stops
once they are read; `first 0` reads nothing. The planner passes the count to
the sources of the pipeline, which may stop early.

The order, the header and the presentation hint of the stream are kept. An
input with fewer records than `count` passes on whole.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `count` | `int` | required | Number of records to keep. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `any` | `same` |

## Effect

| Property | Value |
|---|---|
| Category | `transform` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Keep the first two files:

```nurt example
find "/home/user/project" |> first 2
```

Keep the largest file:

```nurt example
find "/home/user/project" |> sort size order: desc |> first 1
```

Show the first two lines of a log:

```nurt example
lines "/home/user/app.log" |> first 2
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `count` is missing | write a number after `first` |
| `E_INVALID_VALUE` | `count` is negative | use 0 or a larger number |
| `E_TYPE` | `count` is not an integer | write an integer, for example `first 20` |

## Related

`last`, `skip`, `sort`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
