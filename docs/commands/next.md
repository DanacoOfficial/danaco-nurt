---
command: next
group: core
summary: Read the next part of a shortened result.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# next

## Synopsis

```text
next [<id>] [<count>] [name: value]...
```

## Description

`next` returns the next part of a result that an earlier response shortened.
The first response of a command stays short; one more call returns the next
part, then the next, until the end. A response that leaves lines, records,
bytes or the end of long lines out carries the field `continuation` and its
text ends with the hint "more: next".

| Field of `continuation` | Meaning |
|---|---|
| `id` | short identifier, `c1`, `c2`, … within the session of the console |
| `unit` | `lines`, `records` or `bytes` |
| `shown` | units in the responses given so far |
| `remaining` | units not yet given; `-1` when not known in advance |
| `cut_lines` | lines of the shown part that were shortened |

The hint is "more: next" when the continuation is the latest one and
"more: next c3" otherwise. The call forms are:

| Call | Short form | Result |
|---|---|---|
| `next` | `next` | next part of the latest continuation |
| `next id: c3` | `next c3` | next part of the continuation `c3` |
| `next count: 200` | `next 200` | a part of 200 units; the default is the size of the first response |
| `next c3 count: 200` | `next c3 200` | both |
| `next id: c3 all: true` | `next c3 all` | everything that is left of `c3` |
| `next id: c3 full_lines: true` | `next c3 full` | the cut lines of `c3`, whole |
| `next all: true` | `next all` | everything that is left |
| `next full_lines: true` | `next full` | the lines of the last shown part that were cut, whole |

The identifier, the count and the words `all` and `full` are positional; a
bare word `all` or `full` stands for the parameter of the same meaning. The
long forms stay valid and give the same result.

The part has the same record type as the first response of a reading command
and the record type `line` for the output of `run`, so a pipeline continues it:
`next |> filter severity == "error"`. `next` never computes the result again.
It reads the rest from the source that the first response kept:

| Command | Source of the continuation |
|---|---|
| `run` | the output file of the run (lines): the lines between the head and the tail of the response; a selection of lines is continued from the first line of the file |
| `search`, `find`, `list`, `read`, `lines`, `diff`, `git.log`, `git.diff`, `git.blame`, `history`, `processes`, `ports`, `services`, `package.list`, `package.search`, `query`, `sql` and other reading commands | the computed result kept in the state directory, in the order of the response |
| `logs` | the cursor of the query at the time of the first response, so that the parts neither overlap nor skip events that arrive later |
| a pipeline | the result of its last stage |

Details of the parts. `next` after the last lines of a result that cut lines
reads the cut lines whole, as `next full` does; `next full` on a result
without cut lines gives an empty part, and on the continuation of a query with
a cursor (`logs`) it fails with `E_INVALID_VALUE`. A result that shows a
selection of lines (`show: errors`) is continued from the first line of the
file, so its parts repeat the lines already shown. A `run` whose `stdout` and
`stderr` are both shortened continues one stream after the other. When
one program has several stages that cut their output (for example two `run`),
the first of them in program order is continued; the others are reachable
through their own output files. `next` runs a query with a cursor (`logs`)
again under the options of the first call (`session`, `base`, `timeout`). A
continuation belongs to the session that made it: another session gets
`E_CONTINUATION_EXPIRED`. A result larger than `continuation.max_size` is not
kept; the response has no `continuation`, and a notice explains why.

`next` is a reading command: it changes nothing, writes no journal entry and
costs no effect in `E_ONE_EFFECT`. The answer carries its own `continuation`
with the same `id` while something remains; the last part has `remaining: 0`
and no hint. The lifetime and the space of the kept results are set by
`continuation.max_age` (1 hour), `continuation.max_count` (50) and
`continuation.max_size` (256 MiB); the oldest continuation is removed first,
and the results are removed at the end of the session of the console.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `id` | `int \| field \| text` | the latest continuation | Continuation to read, `c3`; also a count (`next 200`) or the words `all` and `full`. |
| `count` | `int \| field \| text` | the size of the first response | Units in the part. |
| `all` | `bool` | `false` | Read everything that is left. |
| `full_lines` | `bool` | `false` | Read the lines of the last shown part that were cut, whole. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `any` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Read the next part of the latest shortened result; without a kept result the
call fails with `E_CONTINUATION_EXPIRED`:

```nurt example
next
```

```text expected
E_CONTINUATION_EXPIRED
```

Read 200 units of the continuation `c3`:

```nurt example
next c3 200
```

Read everything that is left of the latest continuation and count it:

```nurt example
next all |> count
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_CONTINUATION_EXPIRED` | the continuation is older than `continuation.max_age`, was removed by `continuation.max_count` or `continuation.max_size`, or none is kept; `next_steps` holds the call that produced it | run the call from `next_steps` again |
| `E_INVALID_VALUE` | `count` is below 1, or the identifier is not of the form `cN` | give a count of at least 1 and an identifier such as `c3` |

## Related

`run`, `search`, `find`, `list`, `logs`, `help skill: "large-results"`.
