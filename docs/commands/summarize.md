---
command: summarize
group: text
summary: Compute count, sum, minimum, maximum and average of fields, optionally per group.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# summarize

## Synopsis

```text
summarize <fields>... [by: <field>]
```

## Description

`summarize` gives one `summary` record per field in `fields`, and per group
when `by` is given, with the fields `field`, `count`, `sum`, `min`, `max` and
`average`; with `by`, the record starts with the group value under the name
of the `by` field.

`count` is the number of values other than `null`; `null` values are
skipped. Integers and numbers sum to an integer or a number and average to a
number. Sizes and durations sum and average in their own unit, rounded to
whole bytes and milliseconds. For text and other types, and for fields that
mix types, `sum` and `average` are `null` and `min` and `max` follow the
total order of values.

Without `fields`, each group gives one record with the number of its records
in `count`. Groups are ordered by their value. An empty input without `by`
gives one record per field with `count` 0.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `fields` | `list<field>` | - | Numeric fields to summarize. |
| `by` | `field` | - | Field whose value forms the groups. |

## Input and output

| Form | Input | Output |
|---|---|---|
| stage | any record kind | `summary` |

## Effect

Category `transform`, effect `read`, reversibility `read`. The command reads
no files and changes nothing.

## Examples

Compute the total and average size of the files of a project:

```nurt example
find "/home/user/project" |> summarize size
```

Compute the size statistics per extension:

```nurt example
find "/home/user/project" |> summarize size by: extension
```

Count the files per extension:

```nurt example
find "/home/user/project" |> summarize by: extension
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_UNKNOWN_FIELD` | a field is not in the records; `expected` lists their fields | use one of the listed fields |
| `E_LIMIT` | more groups than the limit | group by a field with fewer values |

## Related

`group`, `count`, `distinct`.
