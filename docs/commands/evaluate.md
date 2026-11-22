---
command: evaluate
group: text
summary: Evaluate an expression with arithmetic, times, sizes and built-in functions.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# evaluate

## Synopsis

```text
evaluate EXPRESSION*
```

## Description

`evaluate` computes the value of one expression and returns it with its type.
It starts a pipeline, so the value can be passed to later stages, and it is
the Nurt form of the shell commands that print a computed value (`date`,
`echo`, `Get-Date`, `Write-Output`).

The expression uses the operators and the built-in functions of the language:
arithmetic on integers, numbers, sizes and durations; `instant - instant` is a
duration and `instant +- duration` is an instant; `+` joins texts. Time is
read from the clock of the execution, so `now()` and `ago(7d)` give the time of
the call. The expression is evaluated by the evaluator of the execution; no
other evaluation exists, so `filter`, `select` and `evaluate` give the same
value for the same expression.

The result is one `value` record with the fields `value` and `type`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `expression` | `expression` | required | Expression to evaluate. |

## Input and output

Input: `none`. Output: `value`.

## Effect

Category `source`. Effect `read`. The command changes nothing.

## Examples

Compute an integer.

```nurt example
evaluate 6 * 7
```

```text expected
42
```

Join two texts.

```nurt example
evaluate "Release " + "1.0"
```

```text expected
Release 1.0
```

Compute the time three days ago.

```nurt example
evaluate now() - 3d
```

Compute a size.

```nurt example
evaluate 1.5gib / 3
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_TYPE` | an operand has a type the operator does not accept, for example `"a" - 1` | convert the operand or change the operator |
| `E_MISSING_PARAM` | the expression is missing | give the expression as the first argument |

## Related

`describe`, `filter`, `select`, `help`.
