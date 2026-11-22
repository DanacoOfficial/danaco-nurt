---
command: describe
group: text
summary: List the fields of the input records with their types and an example value.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# describe

## Synopsis

```text
… |> describe
```

## Description

`describe` lists the fields of the records that reach it. It replaces the
inspection of an object with `Get-Member` and answers the question which
fields a following `filter`, `select` or `sort` can use.

The result has one `field_info` record per field:

| Field | Content |
|---|---|
| `field` | name of the field |
| `type` | language type of the values; `mixed` when the records hold values of different types in the field |
| `example` | the first value that is not `null` |
| `required` | `true` when every input record has the field |
| `records` | number of input records that have the field |

Fields appear in the order of their first occurrence, so the order of the
definition of the record kind is kept. Fields that are computed on request
(`hash`, `encoding`, `line_ending`, `mime`, `language`, `binary`, `lines`)
are listed only when the producing command computed them, and then show
`required: false` when some records lack them.

Empty input yields no records. Input that is a single value instead of
records fails with `E_TYPE`. The result is deterministic: the same input gives
the same records in the same order.

## Parameters

`describe` has no parameters.

## Input and output

Input: `any`. Output: `field_info`.

## Effect

Category `transform`. Effect `read`. The command changes nothing.

## Examples

List the fields of the entries of a directory.

```nurt example
list "/home/user/app" |> describe
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app", "type": "directory" },
  { "path": "/home/user/app/a.go", "type": "file", "content_utf8": "package main\n" }
] }
```

```text expected
path
```

Show only the fields that not every record has.

```nurt example
find "/home/user/app" glob: "**/*" |> describe |> filter required == false
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app", "type": "directory" },
  { "path": "/home/user/app/a.go", "type": "file", "content_utf8": "package main\n" }
] }
```

Show the fields of the matches of a search with their types.

```nurt example
search "main" "/home/user/app" |> describe |> select field type
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app", "type": "directory" },
  { "path": "/home/user/app/a.go", "type": "file", "content_utf8": "package main\n" }
] }
```

```text expected
column
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_TYPE` | the input is a single value, or `describe` is the first stage and has no records to describe | put a source before it, for example `list` |

## Related

`help`, `select`, `filter`, `summarize`.
