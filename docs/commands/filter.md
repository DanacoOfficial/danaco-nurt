---
command: filter
group: core
summary: Keep the records that meet a condition.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/filter.md -->
<!-- Responsibility:   Documents filter. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# filter

## Synopsis

```text
any |> filter <condition>
```

## Description

`filter` evaluates `condition` on each input record and passes on the records
for which it is `true`. A condition that evaluates to `null`, for example a
comparison with a field that has no value, counts as `false`. Any other value
than a `bool` stops the program with `E_TYPE`.

When the kinds of the input records are known, an expression that no record
can satisfy is checked before the program runs, by `check` and by the binding
of every run, with the error the evaluation would raise for every record that
has a value in the field (same code, message and `expected`). The check
covers a field of a known type and a literal, in either order, in these
places:

| Expression | Error |
|---|---|
| `size > "abc"`, `modified < "never"`, `size == "abc"`: a text that does not form a value of the type of a `size`, `duration`, `instant`, `path`, `glob` or `key` field | `E_INVALID_VALUE`, with the type in `expected` |
| `<`, `<=`, `>`, `>=` with a literal of another type family (`size > 5`, `name > 5`, `flags > 5`, `raw_name > "abc"`) | `E_TYPE` |
| `in` with a range and a field that is not an `int` (`size in 1..5`), and `in` with a right operand that is not a list, a range or a map (`5 in size`, `name in "abc"`) | `E_TYPE` |
| `+ - * /` on types without a result (`size + "abc"`, `size - 5`, `modified - 5`, `name - 1`) | `E_TYPE` |
| `~` and `!~` with a field that is not text (`flags ~ /a/`) | `E_TYPE` |
| A call of a built-in function whose argument is a field of a type the function does not take (`lower(size)`, `len(exists)`) | `E_TYPE` |
| A call with literal arguments only that the function refuses (`ago("abc")`, `len()`) | the error of the function |

The error is placed at the literal, at the call, or at the whole operation
when both operands are fields. Lists, maps, bytes and ranges take part with
their type family: a literal is never converted to them, so `flags > 5` is
rejected, while `==`, `!=` and `in` with a list or a map never raise an error
on their elements and are not checked. `==` and `!=` with a literal of another
type are not an error and are `false` or `true`.

Only what does not depend on the value of the field is checked. Overflow and
division by zero, an operand that is itself a computed expression, a field of
type `any` (for example `value` of a configuration record) and a field of a
kind that is not known at binding are checked when the program runs, record
by record, and stop it with the same errors. A `null` field value gives
`false` or `null` as before. The condition is checked as written: `and` and
`or` do not hide an error in their right operand from `check`.

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
| default | `any` | `same` |

## Effect

| Property | Value |
|---|---|
| Category | `transform` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

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
| `E_TYPE` | the condition gives a value other than `bool`, or `<`, `<=`, `>` or `>=` compares a field with a literal of another type family | compare the field, for example `size > 1mb` instead of `size`, and write a literal of the type of the field |
| `E_INVALID_VALUE` | a text compared with a `size`, `duration` or `instant` field does not form a value of that type; `expected` names the type | write a valid value, for example `"1mb"`, `"5m"` or `"2026-11-01"` |
| `E_UNKNOWN_FIELD` | the condition names a field the records do not have; `expected` lists their fields | use one of the listed fields |

## Related

`select`, `sort`, `first`, `assert`, `count`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
