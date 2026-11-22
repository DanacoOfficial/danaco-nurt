---
command: analyze.tools
group: verify
summary: List the analysis tools detected for a project and whether they are installed.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# analyze.tools

## Synopsis

```text
analyze.tools [path...] [changed_only: bool]
```

## Description

Lists one `analyze_tool` record for every analyzer and formatter that the
project configuration selects: TypeScript (`tsc`, ESLint, Biome, Prettier),
Python (Ruff, mypy), Go (`go vet`, `gofmt`) and Rust (Clippy, `rustfmt`).
Detection uses configuration files and the extensions of the files in scope.
A program installed in `node_modules/.bin` is preferred to the program on the
search path. Versions are read with `--version`, in parallel.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `list<path>` | the base directory | Files or directories to inspect. |
| `changed_only` | `bool` | `false` | Only files changed in the repository. |

## Input and output

No input. Output: `analyze_tool` records with `tool`, `language`, `check`,
`found`, `version`, `config` and `command`.

## Effect

Category `source`, effect `read`, implementation `builtin`. The version
queries start the tools but change nothing.

## Examples

List the tools of the project.

```nurt example
analyze.tools
```

List the tools for changed files.

```nurt example
analyze.tools changed_only: true
```

List the tools for one directory.

```nurt example
analyze.tools "services/api"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_EXTERNAL_MISSING` | `changed_only` without a repository query service. | Run with `changed_only: false`. |

## Related

`analyze.check`, `analyze.format`.
