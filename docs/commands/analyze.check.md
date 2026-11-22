---
command: analyze.check
group: verify
summary: Check code with type checkers, linters, format checks, complexity and duplication analysis, and return a verdict.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# analyze.check

## Synopsis

```text
analyze.check [path...] [changed_only: bool] [checks: list] [include_tests: bool]
              [complexity_max: int] [fail_on: enum] [min_severity: enum]
              [max_findings: int] [sample_lines: int] [wait: duration]
```

## Description

Runs the analyzers configured in the project and the built-in analyses, merges
their findings and returns the verdict as an `analysis` header followed by
`finding` records. The analysis is stored and can be read again with
`analyze.show`.

Analyzers are detected from configuration files (`tsconfig.json`, ESLint,
Biome, Prettier, `pyproject.toml`, `ruff.toml`, `mypy.ini`, `go.mod`,
`Cargo.toml`) and run with machine-readable output. A program installed in
`node_modules/.bin` is used before the program on the search path. A check
whose tool is not installed has the status `missing` and produces the note
`tool_missing`; the command does not fail.

Complexity and duplication are computed from the data of the code service.
Without the service both checks have the status `missing`.

The verdict is `passed: false` when a finding has the severity of `fail_on` or
a higher one. `min_severity` and `max_findings` limit the returned findings
only; the counts and the verdict cover all findings.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `list<path>` | the base directory | Files or directories to check. |
| `changed_only` | `bool` | `false` | Only files changed in the repository. |
| `checks` | `list<enum(types\|lint\|format\|complexity\|duplication)>` | all available | Checks to run. |
| `include_tests` | `bool` | `true` | Include test files in the duplication check. |
| `complexity_max` | `int` | `15` | Highest cyclomatic complexity accepted. |
| `fail_on` | `enum(error\|warning)` | `error` | Lowest severity that fails the verdict. |
| `min_severity` | `enum(error\|warning\|info)` | `info` | Lowest severity of findings returned. |
| `max_findings` | `int` | `100` | Maximum number of findings returned. |
| `sample_lines` | `int` | `5` | Maximum number of source lines shown with each finding. |
| `wait` | `duration` | `3s` | Time to wait before the analysis continues as a job. |

## Input and output

No input. Output: one `analysis` record, then `finding` records ordered by
severity, check, file and line. A job record is returned when the analysis
continues in the background.

The field `duplication_percent` of the `analysis` record holds the share of
lines of code in the analysed files that belong to a duplicated block, in
both copies, in percent with one decimal. A line counts once however many
blocks cover it. The field is empty when the duplication check did not run.

## Effect

Category `run`, effect `process`, implementation `user_program`. Files that
analyzers write (caches) are detected as for `run` and kept in the journal.

## Examples

Check the whole project.

```nurt example
analyze.check
```

Check changed files for errors and warnings.

```nurt example
analyze.check changed_only: true, fail_on: "warning"
```

Run only the complexity check with a lower threshold.

```nurt example
analyze.check checks: ["complexity"], complexity_max: 10
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_ARG` | Unknown check or `fail_on` value. | Use a value from the parameter table. |
| `E_EXTERNAL_MISSING` | `changed_only` without a repository query service. | Run with `changed_only: false`. |
| `E_PROGRAM_NOT_FOUND` | An analyzer program could not be started. | Install the tool or fix its path. |

## Related

`analyze.show`, `analyze.format`, `analyze.tools`.
