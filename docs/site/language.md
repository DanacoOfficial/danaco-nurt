---
title: Language
order: 4
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Language

The language is defined by the specification: [syntax tree](../../spec/syntax-tree.md), [catalog](../../spec/catalog.md), [records](../../spec/records.md) and [stages](../../spec/stages.md). This page summarizes the rules that the documentation uses.

## Programs and pipelines

A program is a sequence of statements separated by line breaks. A statement is a pipeline or a `let` binding. A pipeline is a command followed by commands joined with `|>`. A line break before `|>` continues the pipeline.

```nurt example
let recent = find "/home/user/app/src" glob: "**/*.ts" |> filter modified > ago(1h)
@recent |> count
```

`let name = pipeline` stores the result under a name and `@name` uses it. There are no variables in text and no substitution of `$` in values.

## Values

| Kind | Written as | Example |
|---|---|---|
| Text | double quotes, escapes `\n \t \\ \"` | `"/home/user/app"` |
| Raw text | `r"…"` or `r#"…"#` | `r#"import "fmt""#` |
| Regular expression | between slashes, flags `i m s x` | `/oldName\(/i` |
| Integer, number | digits, `_` allowed | `1_000`, `1.5` |
| Size | number and unit `b kb mb gb tb kib mib gib tib` | `10mb` |
| Duration | numbers with units `ms s m h d w` | `1h30m` |
| Range | `5`, `1..5`, `40..`, `..9`, `all` | `10..20` |
| List, map | brackets, braces | `["a", "b"]`, `{CI: "1"}` |
| Boolean, null | `true`, `false`, `null` | `true` |

A size written with decimal units (`mb`) counts 1 000 000 bytes for `1mb`; a binary unit (`mib`) counts 1 048 576 bytes.

## Arguments

Positional arguments follow the order of the positions in the catalog. Named arguments are written `name: value` and may appear anywhere after the positional ones. Parameters have types; the [type grammar](../../spec/catalog.md) lists them.

## Expressions

`filter`, `sort`, `select`, `assert` and `evaluate` take expressions evaluated on each record. Operators are `== != < <= > >= ~ !~ in and or not + - * /`. `~` matches a regular expression. Functions include `ago(duration)`.

## Corrections

The console accepts the habits of other shells and forms with small errors. A correction changes the form of the program and never its meaning. Corrections that touch a target of a change are reported with ranked candidates instead of being applied. The habits of bash and PowerShell and the form of each in Nurt are listed on the cheat sheets for [bash](../cheatsheets/nurt-for-bash-users.md) and [PowerShell](../cheatsheets/nurt-for-powershell-users.md).

## Errors

An error has a stable code such as `E_UNKNOWN_PARAM`, a message and, where known, the stage, the position and a replacement program. [Error codes](../../spec/errors.md) lists the codes.
