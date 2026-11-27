---
command: program.describe
group: program
summary: Describe a program with its version, source, description and usage guide.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/program.describe.md -->
<!-- Responsibility:   Documents program.describe. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# program.describe

## Synopsis

```text
program.describe <name> [name: value]...
```

## Description

`program.describe` emits one `program_card` record with the full description of a program, application or usage guide. It starts a pipeline and accepts no input.

The name is looked up in the program index: first as a card name, then as an alias (a symbolic link in `PATH`, a Debian alternative, a shim of Scoop or Chocolatey, an application execution alias, or an alias declared by a catalog), then as the name of a standalone guide of a catalog. For an alias the record describes the main program; the field `requested` holds the given name and `alias_kind` the kind of alias. On Windows and macOS names are compared without regard to case.

The record merges every provider that knows the program: path and alternatives from `PATH`, version, homepage, licence and package description from the package database, application name and identifier from the application entry, the `NAME` line of the manual page in the field `man`, and description, situations of use, tasks the program is not meant for, an example, pitfalls and the usage guide from external catalogs. `guide_sections` lists the headings of the guide; `etag` identifies the content of the record and `guide_etag` that of the guide. Programs that share one guide list each other in `guide_shared_with`.

With a list of names the command emits one record per name that exists and reports the others as warnings; it fails with `E_NOT_FOUND` only when no name exists. `section` selects one section of the guide by heading, `query` the sections of the guide that match its words, and `include_guide: false` leaves the guide out. When `unchanged_since` equals the `etag` of the record, the record carries `unchanged: true` and nothing else of the description. `source` restricts the lookup to the listed sources, named as in `program.catalog`.

The help text of a program is never read on its own: `program.describe` starts no program unless `help: true` is given. The program then runs once with `--help` within the time limit `programs.help_timeout`, and its output is returned in the field `help`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text \| list<text>` | required | Program name, or up to 12 names. |
| `include_guide` | `bool` | `true` | Include the usage guide when one is available. |
| `section` | `text` | none | Return only the section of the guide with this heading. |
| `query` | `text` | none | Return only the sections of the guide that match these keywords. |
| `unchanged_since` | `text` | none | Etag of an earlier record; an unchanged program returns only unchanged. |
| `source` | `text \| list<text>` | none | Only programs from these sources. |
| `help` | `bool` | `false` | Run the program once with --help and return its output in the field help. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `program_card` |

The record carries the fields of `program_card` and, when they apply, `kind`, `etag`, `requested`, `alias_kind`, `description`, `when_to_use`, `not_for`, `example`, `pitfalls`, `man`, `guide`, `guide_sections`, `guide_files`, `guide_etag` and `guide_shared_with`.

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |
| External use | `user_program`; used when `help` is given; Runs the program once with --help, with a time limit, to read its help text. |

## Examples

Describe an installed program.

```nurt example
program.describe "tar"
```

```text expected
name  category  version  path          summary
tar   archives  1.35     /usr/bin/tar  An archiving utility.
```

Describe a program whose guide was already read in the session.

```nurt example
program.describe "ffmpeg" include_guide: false |> select name guide_sections guide_etag
```

Resolve an alias to its main program.

```nurt example
program.describe "vi" |> select name requested alias_kind path
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | No name was given. | Pass a program name. |
| `E_INVALID_VALUE` | The name is longer than 120 characters. | Pass the program name without arguments. |
| `E_NOT_FOUND` | No program, application or guide has the name; `candidates` lists up to eight similar names. | Use a candidate, or find the program with `program.search`. |

## Related

`program.search`, `program.find`, `program.catalog`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
