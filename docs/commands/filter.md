---
command: filter
group: core
summary: Keep the records that meet a condition.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# filter

## Synopsis

```text
filter <condition>
```

## Description

`filter` evaluates `condition` on each input record and passes on the records
for which it is `true`. A condition that evaluates to `null`, for example a
comparison with a field that has no value, counts as `false`. Any other value
than a `bool` stops the program with `E_TYPE`.

The stage is lazy: it reads the next input record only when the following
stage asks for one, so `filter … |> first 10` stops reading the source after
the tenth match. The order of the records, the header of the stream and the
presentation hint of the source are kept. An empty input gives an empty
output.

`filter` works before an effect, where it selects the targets of a change,
and after an effect (stage `5a`), where it reads the records of the effect
such as `exec`, `job`, `test_result`, `finding` and `effect`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `condition` | `expression` | required | Condition evaluated on each record. |

## Input and output

| Form | Input | Output |
|---|---|---|
| stage | any record kind | the input records that meet the condition (`same`) |

## Effect

Category `transform`, effect `read`, reversibility `read`. The command reads
no files and changes nothing.

## Examples

Keep the files larger than 2 kB:

```nurt example
find "/home/user/project" |> filter size > 2kb
```

Keep the lines that set `debug` in a configuration file:

```nurt example
lines "/home/user/app/config.ini" |> filter text ~ /^\s*debug\s*=/
```

Keep the result of a program run only when it failed:

```nurt example
run ["go", "vet", "./..."] cwd: "/home/user/project" |> filter exit_code != 0
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `condition` is missing | write a condition after `filter` |
| `E_TYPE` | the condition gives a value other than `bool` | compare the field, for example `size > 1mb` instead of `size` |
| `E_UNKNOWN_FIELD` | the condition names a field the records do not have; `expected` lists their fields | use one of the listed fields |

## Related

`select`, `sort`, `first`, `assert`, `count`.
