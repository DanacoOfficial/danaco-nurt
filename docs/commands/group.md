---
command: group
group: text
summary: Group records by a field or expression and count each group.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# group

## Synopsis

```text
group <by> [keep_items: <bool>]
```

## Description

`group` computes `by` for each record and gives one `group` record per
distinct value, with the fields `key` (the value) and `count` (the number of
records). With `keep_items: true` the field `items` holds the records of the
group in input order. Values compare by the total order of values.

Groups are ordered by their key, so the output does not depend on the order
of the input; the output declares this order. `group` reads the whole input;
the number of groups, and with `keep_items` the number of held records, is
bounded by the record limit of the request. An empty input gives no groups.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `by` | `field \| expression` | required | Field or expression whose value forms the groups. |
| `keep_items` | `bool` | `false` | Include the records of each group. |

## Input and output

| Form | Input | Output |
|---|---|---|
| stage | any record kind | `group` |

## Effect

Category `transform`, effect `read`, reversibility `read`. The command reads
no files and changes nothing.

## Examples

Count the files of each extension:

```nurt example
find "/home/user/project" |> group extension
```

Group the files by extension and keep them in each group:

```nurt example
find "/home/user/project" |> group extension keep_items: true
```

Split files into small and large ones:

```nurt example
find "/home/user/project" |> group size > 2kb
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `by` is missing | name a field or write an expression |
| `E_UNKNOWN_FIELD` | `by` names a field the records do not have; `expected` lists their fields | use one of the listed fields |
| `E_LIMIT` | more groups or records than the limit | narrow the input or omit `keep_items` |

## Related

`summarize`, `distinct`, `sort`, `count`.
