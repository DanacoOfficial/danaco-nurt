---
command: skip
group: core
summary: Drop the first records.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# skip

## Synopsis

```text
skip <count>
```

## Description

`skip` drops the first `count` input records and passes on the rest. The
stage is lazy: it reads the dropped records one at a time and then passes on
each further record when the next stage asks for it. An error of the source
among the dropped records still stops the program.

The order, the header and the presentation hint of the stream are kept. An
input with no more than `count` records gives an empty output.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `count` | `int` | required | Number of records to drop. |

## Input and output

| Form | Input | Output |
|---|---|---|
| stage | any record kind | the input records after the first `count` (`same`) |

## Effect

Category `transform`, effect `read`, reversibility `read`. The command reads
no files and changes nothing.

## Examples

Drop the first file of a listing:

```nurt example
find "/home/user/project" |> skip 1
```

Drop the header line of a CSV file:

```nurt example
lines "/home/user/data.csv" |> skip 1
```

Dropping more records than the input holds gives no records:

```nurt example
find "/home/user/project" |> skip 5
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `count` is missing | write a number after `skip` |
| `E_INVALID_VALUE` | `count` is negative | use 0 or a larger number |

## Related

`first`, `last`.
