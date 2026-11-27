---
name: large-results
description: Keeping large results small with paging, field selection, counting, summaries and saved output.
commands: [find, filter, select, count, summarize, group, first, sort, run, search, next]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# large-results

## Paging

A result is limited by the call parameter `limit`, 50 records by default, and the answer carries a `cursor` that continues the result in the next call. The program is the same in each call; only `cursor` changes. A limit keeps the answer within the budget of the caller without changing the effect of the program.

## Continuing a shortened result

A response that leaves lines, records or the end of long lines out carries the field `continuation` and ends with the hint "more: next". The call `next` returns the next part, in the same record type as the first response, until `remaining` is 0; the program does not run again and the parts are consistent with the first response.

```nurt
next
next 200
next c3 200
next all
next full
```

`next c3` continues the continuation `c3` when several results are open, `next 200` reads 200 units, `next all` everything that is left and `next full` the lines of the last part that were cut, whole. The part is a result like any other: `next |> filter severity == "error"`. Kept results expire after `continuation.max_age` (1 hour), and the limits `continuation.max_count` (50) and `continuation.max_size` (256 MiB) remove the oldest first; an expired continuation ends with `E_CONTINUATION_EXPIRED` and the original call in `next_steps`.

## Fewer fields

The call parameter `fields` lists the fields of each record to return. `select` does the same inside a program and also adds computed fields, so later stages and the result carry only the fields that are needed.

```nurt
find "/srv/app" glob: "**/*.go" |> select path size
find "/srv/app" glob: "**/*.go" |> select path total: size * 2
```

## Counting and summarizing

`count` returns the number of records and `summarize` computes count, sum, minimum, maximum and average of fields, optionally per group with `by`. `group` counts the records of each value of a field. These stages answer a question about a large set without returning the set.

```nurt
find "/srv/app" |> count
find "/srv/app" |> summarize size by: type
search "deprecated" "/srv/app/src" |> group path
```

## Narrowing early

A condition placed close to the source reduces the data that later stages process. `first` with `sort` returns the largest or newest entries, and `search` with `output: counts` or `output: files` returns counts or file names instead of lines.

```nurt
find "/srv/app" |> sort size order: desc |> first 10
search "deprecated" "/srv/app/src" output: counts
```

## Saved output

The call parameter `save_to` writes the full result to a file and returns a summary with the path of the file. A program that writes a lot of output can limit what returns with `show: errors` or `show: summary`, and `head` and `tail` keep the first or last lines.

```nurt
run ["make", "all"] cwd: "/srv/app" show: errors tail: 40
```
