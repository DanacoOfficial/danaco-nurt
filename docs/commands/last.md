---
command: last
group: core
summary: Keep the last records.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# last

## Synopsis

```text
last <count>
```

## Description

`last` passes on the last `count` input records. The stage reads the whole
input and holds no more than `count` records at a time; when `count` is
larger than the record limit of the request and the input is longer than the
limit, the program stops with `E_LIMIT`.

The order, the header and the presentation hint of the stream are kept. An
input with fewer records than `count` passes on whole.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `count` | `int` | required | Number of records to keep; 0 or more. |

## Input and output

| Form | Input | Output |
|---|---|---|
| stage | any record kind | at most `count` input records from the end (`same`) |

## Effect

Category `transform`, effect `read`, reversibility `read`. The command reads
no files and changes nothing.

## Examples

Keep the last file of a listing:

```nurt example
find "/home/user/project" |> last 1
```

Show the last two lines of a log:

```nurt example
lines "/home/user/app.log" |> last 2
```

Keep all files when there are fewer than ten:

```nurt example
find "/home/user/project" |> last 10
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `count` is missing | write a number after `last` |
| `E_INVALID_VALUE` | `count` is negative | use 0 or a larger number |
| `E_LIMIT` | more records must be held than the limit allows | lower `count` or narrow the input |

## Related

`first`, `skip`, `sort`.
