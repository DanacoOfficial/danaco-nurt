---
command: analyze.show
group: verify
summary: Show the findings of an earlier analysis.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/analyze.show.md -->
<!-- Responsibility:   Documents analyze.show. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# analyze.show

## Synopsis

```text
analyze.show [<analysis_id>] [name: value]...
```

## Description

Returns the stored `analysis` header and the findings of an analysis made by
`analyze.check`. Without `analysis_id` the latest analysis is used. Findings
are filtered by check, by file (a path that contains the given text) and by
severity, then paged with `offset` and `max_findings`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `analysis_id` | `text` | the latest analysis | Earlier analysis. |
| `check` | `text` | none | Only findings of this check. |
| `file` | `path` | none | Only findings in this file. |
| `offset` | `int` | none | Number of findings to skip. |
| `min_severity` | `enum(error\|warning\|info)` | `info` | Lowest severity of findings returned. |
| `max_findings` | `int` | `100` | Maximum number of findings returned. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `finding` |

No input. Output: the `analysis` header and `finding` records.

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Show the findings of the latest analysis.

```nurt example
analyze.show
```

Show lint errors of one analysis in one file.

```nurt example
analyze.show "an-3" check: "lint" file: "src/app.py" min_severity: error
```

Show the second page of findings.

```nurt example
analyze.show offset: 100
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_UNKNOWN_ANALYSIS` | No stored analysis has the identifier; the error lists candidates. | Use an identifier from the candidates or run `analyze.check`. |

## Related

`analyze.check`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
