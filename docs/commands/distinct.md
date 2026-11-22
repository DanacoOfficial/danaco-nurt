---
command: distinct
group: text
summary: Keep one record of each distinct value with the number of occurrences.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# distinct

## Synopsis

```text
distinct [<by>...]
```

## Description

`distinct` keeps the first record of each distinct value of the fields in
`by` and adds the field `count` with the number of records that had this
value. Without `by`, records are distinct by their kind and all their fields.
Values compare by the total order of values.

The first occurrences keep their input order, so an order declared by the
source stays valid. `distinct` reads the whole input; the number of distinct
records is bounded by the record limit of the request. An empty input gives
an empty output.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `by` | `list<field>` | all fields | Fields that make records distinct. |

## Input and output

| Form | Input | Output |
|---|---|---|
| stage | any record kind | the first record of each value with `count` (`same`) |

## Effect

Category `transform`, effect `read`, reversibility `read`. The command reads
no files and changes nothing.

## Examples

Keep one file per extension:

```nurt example
find "/home/user/project" |> distinct extension
```

Count repeated lines of a file:

```nurt example
lines "/home/user/words.txt" |> distinct text
```

List the files with matches once each:

```nurt example
search /error/ "/home/user/logs" |> select path |> distinct
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_UNKNOWN_FIELD` | a field is not in the records; `expected` lists their fields | use one of the listed fields |
| `E_LIMIT` | more distinct records than the limit | narrow the input first |

## Related

`group`, `select`, `sort`.
