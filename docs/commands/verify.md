---
command: verify
group: core
summary: Check a change with a pipeline and roll the change back when the check fails.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# verify

## Synopsis

```text
verify (<pipeline>)
```

## Description

`verify` runs the pipeline `check` after the effect of the program, with the
`effect` records as its input, and reads its whole output. When the check
succeeds, the `effect` records pass on unchanged. When any stage of the check
fails, for example an `assert` inside it, the change is rolled back from the
journal and the program ends with `E_ROLLED_BACK`, which carries the error of
the check; when the rollback cannot restore every file, the error is
`E_ROLLBACK_FAILED` with the files concerned.

The check usually runs a build or a test (`run … |> assert exit_code: 0`) or
reads the changed files again. It runs in stage `6`, after the stages that
read the records of the effect.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `check` | `pipeline` | required | Pipeline that checks the result of the change. |

## Input and output

| Form | Input | Output |
|---|---|---|
| stage | `effect` | `effect` |

## Effect

Category `check`, effect `read`, reversibility `reversible`. A failed check
rolls back the preceding change from the journal. Programs started by the
check are not undone.

## Examples

Replace a call and keep the change only when the project still builds:

```nurt example
search /oldName\(/ "/home/user/project/src" |> replace "newName(" |> verify (run ["go", "build", "./..."] cwd: "/home/user/project" |> assert exit_code: 0)
```

Roll a write back when the file no longer contains a required key:

```nurt example
write "/home/user/project/config.toml" text: "name = 1" mode: overwrite |> verify (lines "/home/user/project/config.toml" |> assert contains: "version")
```

Keep a version change only when the tests pass:

```nurt example
write "/home/user/project/VERSION" text: "1.4.0" mode: overwrite |> verify (run ["make", "test"] cwd: "/home/user/project" |> assert exit_code: 0)
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_ROLLED_BACK` | the check failed and the change was rolled back; the cause holds the error of the check | fix the change or the check and run the program again |
| `E_ROLLBACK_FAILED` | the rollback could not restore every file; the files are listed | restore the listed files with `undo` or by hand |
| `E_MISSING_PARAM` | the check pipeline is missing | write the pipeline in parentheses after `verify` |
| `E_TYPE` | `check` is not a pipeline | enclose the check pipeline in parentheses |

## Related

`assert`, `undo`, `history`.
