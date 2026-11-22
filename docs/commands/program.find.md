---
command: program.find
group: program
summary: Locate programs in PATH, in the order the system resolves them.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# program.find

## Synopsis

```text
program.find <name>... [versions: <bool>]
```

## Description

`program.find` resolves each name in the directories of `PATH` in the order the operating system uses and emits one `program_location` record per name, in the order of the arguments. It starts a pipeline and accepts no input. Repeated names are reported once.

A name that resolves gives `found: true`, the first matching file in `path` and every match in `all_paths`. A name that does not resolve gives a record with `found: false` and no path; the command does not fail. On Windows the extensions of `PATHEXT` are tried in order and names are compared without regard to case; on macOS names are compared without regard to case; on Linux and other Unix systems the comparison is exact and only files with an execute permission are matched. A symbolic link in `PATH` that points to another program in `PATH` is reported with the field `alias_of`.

`program.find` reads only the directories of `PATH`; it does not build the program index unless `versions` is `true`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `list<text>` | required, positional 1, repeated | Program names, without a directory. |
| `versions` | `bool` | `false` | Read the version of each located program from the package that installed it. |

With `versions: true` the version comes from the package databases (dpkg, rpm, pacman, Homebrew, winget, Scoop, Chocolatey) through the mapping of the located file to its package, and `version_source` names the origin. A program that belongs to no known package has no version. No program is started to read its version.

## Input and output

| Form | Input | Output |
|---|---|---|
| `program.find <name>...` | none | `program_location` |

Fields of `program_location`: `name`, `found`, `path`, `all_paths`, `version`, `source`, and the additional fields `alias_of` and `version_source` when they apply.

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | built in; no program is started |

## Examples

Locate two programs.

```nurt example
program.find "git" "rg"
```

```text expected
name  found  path
git   true   /usr/bin/git
rg    true   /usr/local/bin/rg
```

Report the names that are not installed.

```nurt example
program.find "ffmpeg" "gs" "magick" | where found == false | select name
```

Show the installed version of each program with its source.

```nurt example
program.find "python3" "node" versions: true | select name, version, version_source
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | No name was given. | Pass at least one program name. |
| `E_CANCELLED` | The execution was cancelled while names were being resolved. | Run the command again. |

## Related

`program.search`, `program.describe`, `program.catalog`.
