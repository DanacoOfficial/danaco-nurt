---
command: code.profile
group: code
summary: Profile a repository: build and test commands, entry points, change hotspots from history and manifests.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.profile

## Synopsis

```text
code.profile <root> [name: value]...
```

## Description

Describes the project under `root` without running or changing anything in
it. The result is one `profile_entry` record per entry, grouped by section in
the order `languages`, `modules`, `entry_points`, `commands`,
`instructions`, `hotspots`, `manifests`, preceded by a header record.

| Section | Entries |
|---|---|
| `languages` | Languages of the files, by number of files. |
| `modules` | Directories holding a project manifest. |
| `entry_points` | Programs and libraries the project starts from: `main`, `module`, `bin` and `exports` of `package.json`, `[[bin]]` and `src/main.rs` of `Cargo.toml`, `[project.scripts]` of `pyproject.toml`, `cmd/<name>/main.go`, and files with conventional names such as `__main__.py`, `main.py`, `main.go` and `index.js`. |
| `commands` | Build, test and check commands of `package.json` scripts, `Cargo.toml`, `Makefile`, `justfile`, `pyproject.toml`, `setup.cfg`, `tox.ini`, `go.mod`, `CMakeLists.txt`, `build.gradle`, `build.gradle.kts`, `pom.xml`, `*.csproj`, `composer.json`, `Gemfile` and `deno.json`. |
| `instructions` | Files with instructions for contributors and tools (`AGENTS.md`, `CLAUDE.md` and similar). |
| `hotspots` | Files with the most commits in the last `days` days, then files with uncommitted changes. Outside a repository, the most recently modified files. Dependency lock files and build output are left out. |
| `manifests` | Every manifest found. |

Every command and entry point names its origin: the manifest, followed by
the key or the line it was read from (`package.json:scripts.test`,
`Makefile:12`). Paths are absolute.

JSON and TOML manifests are read through the document service; without it,
JSON manifests are read directly and TOML manifests contribute only the
commands their presence implies, with a note in the report. History and
status come from the repository query service.

The profile is cached in the state directory. A cached profile is used while
it is younger than ten minutes, the repository is on the same commit, the
history period is the same and no manifest it was read from has changed.
Uncommitted changes are read on every call. `refresh: true` reads the tree
again.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `root` | `path` | required | Repository root. |
| `sections` | `list<enum(commands\|entry_points\|hotspots\|manifests\|languages\|modules\|instructions)>` | none | Sections to return; all when omitted. |
| `limit` | `int` | none | Maximum number of entries per section. |
| `days` | `int` | `90` | Days of history used for change hotspots. |
| `refresh` | `bool` | `false` | Rebuild the profile instead of using the cached one. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `profile_entry` |

The header record has the fields `root`, `revision` (commit, or null outside
a repository), `repository`, `files` (number of files listed), `truncated`
(the listing stopped at 50 000 files), `cached` and, for a cached profile,
`cache_age_ms`.

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Profile the project in `/srv/app`.

```nurt example
code.profile "/srv/app"
```

```text expected
section       name    command         origin
commands      build   npm run build   package.json:scripts.build
commands      test    npm run test    package.json:scripts.test
```

List the build and test commands only.

```nurt example
code.profile "/srv/app" sections: [commands]
```

Show the five files changed most often in the last 30 days.

```nurt example
code.profile "/srv/app" sections: [hotspots] days: 30 limit: 5 |> select name changes last_change
```

Read the profile again after a change of the build setup.

```nurt example
code.profile "/srv/app" refresh: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `root` is not a directory. | Pass the root directory of a project. |
| `E_INVALID_VALUE` | `sections`, `limit` or `days` is out of range. | Use the values listed under Parameters. |
| `E_MISSING_PARAM` | `root` is missing. | Pass the root directory. |

## Related

`code.languages`, `code.symbols`, `code.outline`, `git.log`.
