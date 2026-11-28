---
command: sort
group: core
summary: Sort the records by fields.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/sort.md -->
<!-- Responsibility:   Documents sort. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# sort

## Synopsis

```text
any |> sort <by>... [name: value]...
```

## Description

`sort` orders the records by one or more keys, the first key first. Keys are
fields or expressions. Values compare by the total order of values: values of
different types order by type (`null` first, then `bool`, numbers, sizes,
durations, instants, text, paths and the other types), so a mixed field sorts
deterministically. Text compares by Unicode code points.

The sort is stable: records with equal keys keep their input order, also with
`order: desc`. `sort` reads the whole input and holds it in memory within the
record limit of the request; a larger input stops the program with `E_LIMIT`.
When the source already produces the records in the requested order, the
stream passes through unchanged. The output declares its order, so a
following `sort` by the same fields does not sort again. An empty input gives
an empty output.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `by` | `list<field>` | required | Fields to sort by, in order of priority. |
| `order` | `enum(asc\|desc)` | `asc` | Sort order. |

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

Sort files from the largest:

```nurt example
find "/home/user/project" |> sort size order: desc
```

Sort by extension, then by size within each extension:

```nurt example
find "/home/user/project" |> sort extension size
```

Sort the lines of a file alphabetically:

```nurt example
lines "/home/user/data.txt" |> sort text
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | no key is given | name at least one field |
| `E_UNKNOWN_FIELD` | a key names a field the records do not have; `expected` lists their fields | use one of the listed fields |
| `E_UNKNOWN_VALUE` | `order` is neither `asc` nor `desc` | write `order: asc` or `order: desc` |
| `E_LIMIT` | the input holds more records than the limit | narrow the input with `filter` first |

## Related

`first`, `last`, `group`, `distinct`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
