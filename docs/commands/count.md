---
command: count
group: core
summary: Count the records.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# count

## Synopsis

```text
count
```

## Description

`count` reads the whole input and gives the number of records as an `int`.
The records are not held in memory. An empty input gives 0.

`count` counts file records before an effect and the records of an effect
after it (stage `5a`), for example the `exec` records of a program run.

## Parameters

The command has no parameters.

## Input and output

| Form | Input | Output |
|---|---|---|
| stage | any record kind | `int` |

## Effect

Category `transform`, effect `read`, reversibility `read`. The command reads
no files and changes nothing.

## Examples

Count the files of a project:

```nurt example
find "/home/user/project" |> count
```

Counting the lines of an empty file gives 0:

```nurt example
lines "/home/user/empty.txt" |> count
```

Count the results of a program run:

```nurt example
run ["git", "status", "--short"] cwd: "/home/user/project" |> count
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_TYPE` | the input is a single value, not records | apply `count` to a stage that produces records |

## Related

`assert`, `summarize`, `group`.
