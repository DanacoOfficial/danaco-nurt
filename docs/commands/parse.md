---
command: parse
group: text
summary: Extract fields from text with the named groups of a regular expression.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# parse

## Synopsis

```text
line|match|content |> parse <pattern>
```

## Description

`parse` matches `pattern` against the text of each record and gives a `row`
record whose `cells` are the named groups of the match, as text; a group that
does not take part in the match is `null`. Groups are named with
`(?<name>…)`. The row keeps the `path` of its input record.

| Input | Text | Rows |
|---|---|---|
| `line` | `text` | one row for the first match, numbered by the line |
| `match` | `line`, the whole line of the match | one row for the first match, numbered by the line |
| `content` | `text` | one row per match, numbered by the line where the match starts |

Records the pattern does not match give no row. The stage is lazy and keeps
the header of the stream.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `pattern` | `regex` | required | Pattern whose named groups become fields. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `line`, `match`, `content` | `row` |

## Effect

| Property | Value |
|---|---|
| Category | `transform` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Split log lines into a level and a message:

```nurt example
lines "/home/user/app.log" |> parse /(?<level>[A-Z]+) (?<message>.*)/
```

Read the settings of an environment file:

```nurt example
read "/home/user/app.env" |> parse /^(?<key>\w+)=(?<value>.*)$/m
```

Extract a name and a number from the lines found by a search:

```nurt example
search /port/ "/home/user/app.conf" |> parse /(?<name>\w+)\s*=\s*(?<value>\d+)/
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | the pattern has no named groups | name the groups, for example `(?<level>\w+)` |
| `E_REGEX` | the pattern does not compile | correct the pattern |
| `E_TYPE` | the input records are not `line`, `match` or `content` | read the text first with `lines`, `search` or `read` |

## Related

`split`, `search`, `lines`.
