---
command: assert
group: core
summary: Stop the program with E_ASSERT unless the records meet one condition.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# assert

## Synopsis

```text
any |> assert (count: <range> | paths: <range> | exit_code: <range> | all: <expression> | any: <expression> | empty: <bool> | contains: <text> | if_match: <text>)
```

## Description

`assert` reads the whole input, checks exactly one condition and passes the
records on unchanged, with their header and order, when the condition holds.
When it does not hold, the program stops with `E_ASSERT`; the error carries
`expected`, `actual` and a `sample` of up to five records or paths.

| Condition | Holds when |
|---|---|
| `count` | the number of records is in the range |
| `paths` | the number of distinct `path` values is in the range |
| `exit_code` | every record has an `exit_code` in the range; fails when no program ran |
| `all` | the expression is `true` for every record |
| `any` | the expression is `true` for at least one record |
| `empty` | the input has no records (`true`) or has records (`false`) |
| `contains` | a text field of a record contains the text |
| `if_match` | every input record has the given `etag`; fails without input |

Before an effect (stage `3`) a failed assertion changes nothing. After an
effect (stage `5a`), for example after `run`, the effect stays, the report is
complete and the error has the stage `5a`; only `verify` rolls an effect
back.

A range is written as `5`, `1..5`, `40..` or `..9`; both bounds are included.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `count` | `range` | one of `condition` | Expected number of records. |
| `paths` | `range` | one of `condition` | Expected number of distinct paths. |
| `exit_code` | `range` | one of `condition` | Expected exit code of the programs run. |
| `all` | `expression` | one of `condition` | Condition that every record meets. |
| `any` | `expression` | one of `condition` | Condition that at least one record meets. |
| `empty` | `bool` | one of `condition` | Whether there are no records. |
| `contains` | `text` | one of `condition` | Text that the records contain. |
| `if_match` | `text` | one of `condition` | Expected etag of the input file. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `any` | `same` |

## Effect

| Property | Value |
|---|---|
| Category | `check` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Check that a search found exactly two files before they are changed:

```nurt example
search /oldName\(/ "/home/user/project/src" |> assert paths: 2
```

Stop when the number of files differs from the expected one:

```nurt example
find "/home/user/project" glob: "*.md" |> assert count: 2
```

Check the exit code of a program after it ran:

```nurt example
run ["go", "test", "./..."] cwd: "/home/user/project" |> assert exit_code: 0
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_ASSERT` | the condition does not hold; `expected`, `actual` and `sample` describe it | correct the input or the expected value |
| `E_MISSING_PARAM` | no condition is given | give one of the parameters |
| `E_PARAM_CONFLICT` | more than one condition is given; `fix` lists each alone | keep one condition per `assert`, chain several stages |
| `E_TYPE` | a condition has a value of another type, or `all`/`any` gives a value other than `bool` | use the type of the parameter |

## Related

`verify`, `count`, `filter`.
