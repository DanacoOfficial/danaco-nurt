---
group: program
summary: "Programs installed in the system: lookup, search, navigation and descriptions."
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Group program

## Scope

The group answers which programs exist on the computer and how to use them.
It locates programs in the search path, searches installed programs and the
program catalogs, describes a program with its version and usage guide, and
browses programs by category.

The group works on every computer from the first call and needs no
configuration. External catalogs are optional and add descriptions, categories
and usage guides to the programs that the system already reports.

| Task | Command | Group |
|---|---|---|
| Run a program | `run` | core |
| List, install and remove packages | `package.*` | package |
| Describe the host and its operating system | `host` | system |
| Read the manual page of a program as a file | `read` | core |

`package.search` queries the repositories of a package manager and needs the
manager. The commands of this group read the databases of the package managers
and never start the manager.

## Commands

| Command | Category | Input and output | Reversibility | Page |
|---|---|---|---|---|
| `program.find` | source | none to `program_location` | read | [program.find](../commands/program.find.md) |
| `program.search` | source | none to `program_card` | read | [program.search](../commands/program.search.md) |
| `program.describe` | source | none to `program_card` | read | [program.describe](../commands/program.describe.md) |
| `program.catalog` | source | none to `program_category` or `program_card` | read | [program.catalog](../commands/program.catalog.md) |

## Records

| Record | Required fields |
|---|---|
| `program_location` | `name`, `found` |
| `program_card` | `name`, `summary`, `source`, `installed`, `guide_optional`, `sources` |
| `program_category` | `name`, `count`, `source` |

`program_location` reports the path that a name resolves to; `all_paths` lists
every match in the order the system resolves them. `program_card` carries the
description in `description` and the usage guide in `guide`. The field `source`
names the main source (`path`, `package`, `app` or `catalog`) and `sources`
lists every source from which the card was assembled, so the origin of each
text is known.

## Sources of data

| Source | Value of `source` | Content |
|---|---|---|
| Search path | `path` | name, path and version of executable files; `PATHEXT` is honored on Windows |
| Package databases | `package` | name, version, description and executables of installed packages |
| System applications | `app` | name, path, version and publisher of applications |
| Program catalogs | `catalog` | descriptions, categories and usage guides |
| Descriptions | | manual pages and `whatis` entries, package metadata |

The package databases are read from files, without starting the manager:

| System | Databases |
|---|---|
| Linux | dpkg, rpm, pacman |
| macOS | Homebrew |
| Windows | winget, Scoop, Chocolatey |

Applications come from the registry and the Start menu on Windows, from
application bundles on macOS and from desktop entries on Linux.

## External catalogs

A catalog is a directory in the format `nurt-program-catalog/1`:

```text
catalog.json        index: format, categories and programs
guides/<name>.md    usage guide of a program, Markdown with a front matter
```

The index lists categories (`id`, `title`, `description`, `parent`) and
programs (`name`, `category`, `summary`, `aliases`, `platforms`, `guide`,
`verified`, `tags`). The reader treats a catalog as data: it executes no field
and no file of the catalog and fetches no address.

The setting `programs.catalogs` lists the catalogs: paths or tables with
`path`, `priority` and `enabled`. The default is an empty list, and the product
names no catalog. Without catalogs the commands describe the installed programs.
A catalog that is configured explicitly and cannot be read fails with
`E_CATALOG`. The setting `programs.sources_disabled` excludes sources
(`path`, `package`, `app`, `man`, `catalog`) from the index.

## Platform differences

| Aspect | Linux | macOS | Windows |
|---|---|---|---|
| Executable files | files with the execute permission | files with the execute permission | extensions of `PATHEXT` |
| Application source | desktop entries | application bundles | registry and Start menu |
| Package databases | dpkg, rpm, pacman | Homebrew | winget, Scoop, Chocolatey |
| Version of an executable | package version | bundle version | version resource of the file |

`E_OS_ONLY` is not reported by this group: every command works on all
three systems and a source that does not exist on the system is skipped.

## External programs

| Command | Marker | Condition |
|---|---|---|
| `program.describe` | [U] | runs the program once with `--help`, within a time limit, when no other description is available |

The time limit of reading the help is the setting `programs.help_timeout`
(default `3s`). No other command of the group starts a program, and the
index is never built by starting programs. The [U] marker names a program of
the user.

## Examples

Locate two programs and report their versions:

```nurt example
program.find ["git", "python3"] versions: true
```

Search installed programs and catalogs, and keep the installed ones:

```nurt example
program.search "convert images" limit: 10 |> filter installed |> select name path summary
```

Search only the search path and the package databases:

```nurt example
program.search "pdf" source: [path, package] limit: 5
```

Describe a program without its usage guide:

```nurt example
program.describe "ffmpeg" include_guide: false
```

List the categories with the most programs:

```nurt example
program.catalog |> sort count order: desc |> first 5
```

## Related

[Group package](package.md) installs the programs that this group finds.
[Group system](system.md) reports the host on which the search runs.
