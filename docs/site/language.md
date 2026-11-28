---
title: Language
order: 4
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/language.md -->
<!-- Responsibility:   Documents Language. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
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

## Conditions

`when` runs one branch. The condition is a `bool` expression: comparisons, `and`, `or`, `not`, `exists <path>`, a reading pipeline in parentheses (`(processes name: "nginx" |> count) > 0`), `exit_code` and the fields of a record held by a `let`. `otherwise when` repeats and `otherwise` is optional. The condition is evaluated once, before its branch; the report gives its value and the number of the branch. A condition that is not `bool` is `E_TYPE` before any effect when its type is known at binding: a comparison, a bare pipeline, `exit_code`, and a field of the loop record whose type the output record of the source command declares. The same holds for every use of such a field that no value of its type can satisfy: `f.size ~ "x"`, `f.name < 3`, `f.name == 3`, `not f.name`, `f.path - 1`, `1 in f.size`. The only exception is a field typed `any` or `json`, whose type depends on the data: it is checked when the condition is evaluated, and a failure then restores the construct. A pipeline in a condition only reads: a stage that may change the world and takes its action from a value that is not a literal (`http.request … method: @m`, `git.branch delete: @a`) is `E_TYPE` at binding.

```nurt example
when (read "/etc/app.toml" key: "mode") == "prod" {
    replace /debug = true/ "debug = false" in: "/etc/app.toml" expect: 1
} otherwise when exists "/srv/app/.staging" {
    write "/srv/app/.ready" "staging\n" mode: overwrite
} otherwise {
    write "/srv/app/.dev" "1\n" mode: overwrite
}
```

## Loops

`each <records> as name { … }` runs the block once per record, in order; `name` is the record and `name.path` a field. `repeat until: <bool> every: <duration> limit: <duration or number> { … }` runs the block and then tests the condition; `while <bool> limit: <duration or number> { … }` tests before each run. Without `limit` a loop ends at `loop.max_iterations` (10 000) or `loop.max_time` (10 minutes): `repeat` then ends with `E_LIMIT`, `while` ends normally and the report says that the limit ended it. With `limit` the loop ignores the configured defaults: a number of iterations is then not bounded by time, and a duration is not bounded by the number of iterations. `preview` plans one iteration of a `repeat` or a `while`, because the number of iterations is known only by running; an `each` plans every iteration. `and` and `or` evaluate both operands, so a pipeline in the right operand also runs when the left one decides. `break` and `continue` work inside a loop, usually after `when`. `each` accepts `parallel: N`; every loop accepts `atomic: false`, which keeps the iterations completed before a failure and names the failed one.

```nurt example
each (find "/srv/app" glob: "**/*.log") as f {
    lines f.path |> filter text ~ /ERROR/ |> write "/srv/reports/" + f.name + ".err" mode: overwrite
}
```

## Several changes in one call

`steps { … }` holds pipelines that each change; `let` passes results to later steps.

```nurt example
steps {
    find "/srv/app/old" glob: "*.conf" |> move to: "/srv/app/conf"
    replace /old\/path/ "conf/path" in: "/srv/app/app.toml" expect: 3
}
```

## Functions

`define name(parameter: type = default, …) { … }` defines a function; a call is written `name(value, parameter: value)`. The body sees its parameters (`@dir`) and its own `let`, nothing of the caller. The value of the last statement is the result. Recursion ends with `E_LIMIT` at `function.max_depth` (64).

```nurt example
define rotate_logs(dir: path, keep: int = 7) {
    find @dir glob: "*.log.*" |> sort modified order: desc |> skip @keep |> remove
}
rotate_logs("/var/log/app1")
rotate_logs("/var/log/app2", keep: 14)
```

## Script parameters

A file starts with `params { … }`; each line is a name, a type and an optional default. `nurt run cleanup.nurt dir: "/srv/app" days: 7 dry` passes the values, `nurt run cleanup.nurt --help` prints the declaration, and the engine takes the values in the `params` member of the request. A missing value, an unknown name (with the nearest name) or a value of the wrong type is reported before anything runs.

```nurt example
params {
    dir: path
    days: int = 30
    dry: bool = false
}
find @dir glob: "**/*.log" |> filter modified < ago(@days * 1d) |> remove preview: @dry
```

## Rules of every construct

A construct is one journal entry: one `undo` reverses all of it. It is all or nothing: when a part fails, the changes already made are rolled back, and the parts that cannot be rolled back (a program run, a restart) are listed before the change and named in the error. `preview` computes the chosen branch, the iterations and every effect without a change. Types, names and arity are checked before any effect, and constructs are part of the syntax tree: no text is evaluated.

## Boolean shorthand

A parameter of type `bool` written without a value is `true`: `remove "/tmp/x" preview`, `nurt run cleanup.nurt dry`. `false` is always written out (`preview: false`). A parameter that is not `bool` written without a value is `E_MISSING_PARAM` with the expected type.

## Corrections

Nurt accepts the habits of other shells and forms with small errors. A correction changes the form of the program and never its meaning. Corrections that touch a target of a change are reported with ranked candidates instead of being applied. The habits of bash and PowerShell and the form of each in Nurt are listed on the cheat sheets for [bash](../cheatsheets/nurt-for-bash-users.md) and [PowerShell](../cheatsheets/nurt-for-powershell-users.md).

## Errors

An error has a stable code such as `E_UNKNOWN_PARAM`, a message and, where known, the stage, the position and a replacement program. [Error codes](../../spec/errors.md) lists the codes.

## Choosing a construct

| The task | Write | Journal and failure |
|---|---|---|
| one change | a pipeline with one changing stage at its end | one entry |
| several changes that stand or fall together | `steps { … }` | one entry, all or nothing |
| a change that depends on the state found | `when … otherwise …` | one entry with the chosen branch |
| the same change for many records | `each … as name { … }` | one entry; `atomic: false` keeps the completed iterations |
| poll for a state, or repeat until a limit | `repeat until:` or `while`, with `limit` | one entry; the limit ends with `E_LIMIT` in `repeat` |
| a change used several times in a file | `define name(…) { … }` | part of the entry of the caller |
| a file that takes values | `params { … }`, then `nurt run file.nurt name: value` | checked before any effect |

A single pipeline that has more than one changing stage is `E_ONE_EFFECT`; the correction proposes the same pipelines inside `steps` as the first candidate. The mode `script: true` stays for scripts that want a separate journal entry for every pipeline.

## Where the other features are described

- `sql` changes a database without `write: true`, accepts `BEGIN`, `COMMIT`, `ROLLBACK`, `ATTACH`, `VACUUM` and `VACUUM INTO`, and journals every database file it changes: see the [command](../commands/sql.md).
- `create` makes named pipes (`type: fifo`) and device nodes (`type: device`): see the [guide for Linux and macOS](../guides/unix.md).
- Accounts, groups, mounts, kernel parameters, firewall, network, clock, power and disks, and elevated work with `privilege` and `as_user`: see the [administration guide](../guides/administration.md).
- The habits of bash and PowerShell and their form in Nurt: see the guides for [bash](../guides/bash-migration.md) and [PowerShell](../guides/powershell-migration.md).

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
