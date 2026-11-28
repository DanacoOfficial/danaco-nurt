---
command: analyze.format
group: verify
summary: Format source files with the formatter configured for each language.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/analyze.format.md -->
<!-- Responsibility:   Documents analyze.format. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# analyze.format

## Synopsis

```text
analyze.format <path>... [name: value]...
```

## Description

Finds the files that the configured formatters would change, then runs the
formatters on exactly those files. The formatters are Prettier, Biome, Ruff,
`gofmt` and `rustfmt`. The set of files is determined before any formatter
starts: each formatter is first run in its check mode on the requested files.

The result lists the files and shows up to `sample_lines` changed lines per
file as `-old` and `+new` lines.

Files changed outside the determined set are detected as for `run`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `list<path>` | none | Files or directories to format. |
| `changed_only` | `bool` | `false` | Only files changed in the repository. |
| `sample_lines` | `int` | `12` | Maximum number of changed lines shown per file, from 1 to 200. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

No input. Output: an effect report with one change per formatted file.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `process` |
| Reversibility | `reversible` |
| Implementation | `user_program` |
| Reversibility conditions | The previous content of every file passed to the formatter is kept in the journal. |

## Examples

Format files changed in the repository.

```nurt example
analyze.format changed_only: true
```

Format one directory and show more changed lines.

```nurt example
analyze.format "src" sample_lines: 40
```

Format two directories.

```nurt example
analyze.format "src" "tests"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_SPAWN` | A formatter ended with a failure. | Run `analyze.check checks: [format]` and fix the reported syntax errors. |
| `E_PROGRAM_NOT_FOUND` | A formatter program could not be started. | Install the formatter or fix its path. |

## Related

`analyze.check`, `analyze.tools`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
