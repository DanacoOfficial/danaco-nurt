<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Naming conventions

This document defines the names of the product, of its artifacts and of the
elements of the language, and the rules for choosing new names.

## Product name

| Element | Name |
|---|---|
| Product | Danaco Nurt (in running text: Nurt) |
| Description line | Danaco Nurt — command console with typed pipelines and per-operation undo |
| Executable | `nurt` (`nurt.exe` on Windows); no other names or aliases are installed |
| Pronunciation | NOORT |

The names "Danaco" and "Danaco Nurt" are names of Danaco Holding Group
Sp. z o.o. The Apache License 2.0 does not grant permission to use them
(Section 6 of the License), except for the customary description of the origin
of the product and the reproduction of the `NOTICE` file. Descriptive
statements such as "works with Danaco Nurt" are customary use. A modified
version of the program must not be distributed under the name Danaco Nurt
without the permission of Danaco Holding Group Sp. z o.o.

## Identifiers derived from the product name

| Element | Identifier |
|---|---|
| Program files | extension `.nurt`, UTF-8; optional first line `#!/usr/bin/env nurt` |
| Syntax tree in JSON | extension `.nurt.json` |
| Media types | `application/vnd.nurt.ast+json`, `application/vnd.nurt.ast+cbor`, `application/vnd.nurt.report+json`, `application/vnd.nurt.request+json` |
| C library | `libnurt` (`libnurt.so`, `libnurt.dylib`, `nurt.dll`); static `libnurt.a`, `nurt.lib`; SONAME `libnurt.so.<ABI major>` |
| C header | `nurt.h` |
| C functions and types | prefix `nurt_`, in the form `nurt_<verb>_<noun>` (for example `nurt_run_json`) |
| C macros | prefix `NURT_` |
| Environment variables | prefix `NURT_`, upper case (for example `NURT_CONFIG`) |
| Configuration file | `nurt.toml`; project configuration `.nurt.toml` |
| Journal directory | `.nurt-journal` (`.nurt-journal.noindex` on macOS) |
| User state directory | Linux `$XDG_STATE_HOME/nurt/`, macOS `~/Library/Application Support/Nurt/`, Windows `%LOCALAPPDATA%\Nurt\` |
| Service | `nurt service run`; systemd unit `nurt-service.service`; Windows service `NurtService` (display name "Danaco Nurt Service"); launchd label `nurt.service` |
| Tool name in integrations and in the MCP server | `nurt` |
| Rust crates | `nurt-<area>` (for example `nurt-journal`, `nurt-git-ops`) |

## Names in the language

The vocabulary of the language uses standard computing terms and ordinary
English words. A name is accepted when it:

1. is a common computing term or an ordinary English word;
2. is not an abbreviation associated with a single program (for example
   `grep`, `sed`, `awk`, `rm`, `ls`, `mv`, `cp`, `cat`); verbs that are both an
   ordinary word and a program name (`find`, `sort`, `diff`) are permitted;
3. is not an invented term or a metaphor;
4. is unambiguous within the catalog: no two command names are at an edit
   distance of 1, except for documented pairs;
5. is known from widespread use in operating systems, programming languages,
   SQL and editors.

Names of other products are not used as names of commands or parameters.
Command groups may be named after the technology they operate on (for example
the `git.` group for the version control system).

| Element | Form | Examples |
|---|---|---|
| Commands | lower case, `snake_case`; groups separated by a dot | `find`, `replace`, `git.status` |
| Parameters | lower case, `snake_case` | `path`, `glob`, `depth`, `order` |
| Enumeration values | lower case | `asc`, `desc`, `directory`, `overwrite` |
| Record types | lower case | `entry`, `line`, `match`, `job` |
| Error codes | `E_` followed by upper case words | `E_NOT_FOUND`, `E_ASSERT` |
| Configuration keys | `section.key`, lower case `snake_case` | `journal.max_ops` |

Full words are preferred to abbreviations (`directory`, not `dir`; `instant`,
not `timestamp`). Abbreviations are used only when they are the standard term
(`lf`, `crlf`, `json`, `regex`).

## Names in the code

Rust code follows the Rust API Guidelines: `UpperCamelCase` for types and
traits, `snake_case` for functions, methods, modules and variables,
`SCREAMING_SNAKE_CASE` for constants and statics. Types of the contract crate
that correspond to language elements use the name of the element (for example
the record type `entry` is the type `Entry`). Serialized field names are
`snake_case`.
