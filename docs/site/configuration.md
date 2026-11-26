---
title: Configuration
order: 8
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Configuration

The format is `nurt-config/1`. The complete list of keys with types and defaults is in the [specification](../../spec/configuration.md); the schema is printed by `nurt schema config`.

## Files

| System | System file | User file | State directory |
|---|---|---|---|
| Linux | `/etc/nurt/nurt.toml` | `$XDG_CONFIG_HOME/nurt/nurt.toml` (default `~/.config/nurt/nurt.toml`) | `$XDG_STATE_HOME/nurt/` |
| macOS | `/Library/Application Support/Nurt/nurt.toml` | `~/Library/Application Support/Nurt/nurt.toml` | `~/Library/Application Support/Nurt/` |
| Windows | `%PROGRAMDATA%\Nurt\nurt.toml` | `%APPDATA%\Nurt\nurt.toml` | `%LOCALAPPDATA%\Nurt\` |

A project may contain `.nurt.toml`.

## Precedence

Later levels override earlier ones:

1. built-in defaults
2. the system file
3. the user file
4. `.nurt.toml` of the project
5. the file named by `NURT_CONFIG`
6. the variables `NURT_*`
7. the parameters of the call

## Project files

`.nurt.toml` may set `scope.base`, `scope.project_roots`, `report.*` and `run.*`, and `journal.*` for trusted projects only. The keys `scope.trusted_projects`, `scope.trust*` and `service.*` in a project file are ignored and reported as `trust_config_ignored`.

## Keys used in the documentation

| Key | Variable | Default | Meaning |
|---|---|---|---|
| `syntax.variant` | `NURT_SYNTAX` | `a` | variant of the text syntax |
| `syntax.tolerant` | `NURT_SYNTAX_TOLERANT` | `true` | accept the syntax of the other variants |
| `correction.enabled` | `NURT_CORRECTION` | `true` | corrections of the form |
| `journal.enabled` | `NURT_JOURNAL` | `true` | journal of changes |
| `journal.max_ops` | `NURT_JOURNAL_MAX_OPS` | `30` | operations kept |
| `journal.max_age` | `NURT_JOURNAL_MAX_AGE` | `"30m"` | age of the operations kept |
| `run.timeout` | `NURT_RUN_TIMEOUT` | `"120s"` | time limit of `run` |
| `output.language` | `NURT_LANGUAGE` | `"en"` | language of the fixed texts of `help` and `describe` (`en` or `pl`); error messages stay in English |
| `output.color` | `NURT_COLOR` | `"auto"` | colors for a person |
| `session.id` | `NURT_SESSION` | none | session identifier in the journal |
