---
command: split
group: text
summary: Split lines into columns at a delimiter.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# split

## Synopsis

```text
line |> split <delimiter> [name: value]...
```

## Description

`split` divides the `text` of each `line` record at `delimiter` and gives a
`row` record whose `cells` are the parts, as text. A text delimiter is
matched literally; a regular expression matches any delimiter it describes,
such as `/\s+/` for runs of white space.

Cells are named by `names` in order. Parts beyond the names are named
`c<n>` by their position from 1, and names beyond the parts get `null`.
Without `names` the cells are `c1`, `c2`, and so on. The row keeps the `path`
and the line number `no` of its line. The stage is lazy and keeps the header
of the stream.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `delimiter` | `text \| regex` | required | Delimiter text or pattern. |
| `names` | `list<text>` | none | Names of the columns. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `line` | `row` |

## Effect

| Property | Value |
|---|---|
| Category | `transform` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Split CSV lines into numbered columns:

```nurt example
lines "/home/user/data.csv" |> split ","
```

Split CSV lines after the header into named columns:

```nurt example
lines "/home/user/data.csv" |> skip 1 |> split "," names: ["id", "name"]
```

Split columns separated by white space:

```nurt example
lines "/home/user/table.txt" |> split /\s+/ names: ["user", "shell"]
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | the delimiter is an empty text | give at least one character |
| `E_REGEX` | the delimiter pattern does not compile | correct the pattern |
| `E_TYPE` | the input records are not `line`, or `names` is not a list of text | read the file with `lines`; write names as `["a", "b"]` |

## Related

`parse`, `lines`, `skip`.
