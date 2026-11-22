---
title: Danaco Nurt
order: 1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Danaco Nurt

Danaco Nurt (`nurt`) is a command shell and language where every change can be undone.

The language works with files, text, structured documents, source code, version control, processes and the operating system. A program is a pipeline of typed records. Each program is checked before it runs, and each run returns a report of its effects.

The command line program `nurt` runs programs in a terminal and in scripts. Other programs call the same engine as a tool through the Model Context Protocol, a JSON-RPC server or a library.

## Documentation

| Part | Content |
|---|---|
| [Installation](installation.md) | Obtaining and verifying the executable |
| [First steps](first-steps.md) | Running a program, reading a result, undoing a change |
| [Language](language.md) | Syntax, values, pipelines and corrections |
| [Command line](command-line.md) | The `nurt` program and its subcommands |
| [Effect report](effect-report.md) | Fields of the report of a run |
| [Journal and undo](journal.md) | Recording, undoing and redoing changes |
| [Configuration](configuration.md) | Files, levels and keys |
| [Integration](integration.md) | Tool definitions, MCP and libraries |
| [Commands](../commands/README.md) | One page for each of the 148 commands |
| [Cookbook](../cookbook/README.md) | Programs for common tasks |
| [Cheat sheets](../cheatsheets/nurt-for-bash-users.md) | Habits of bash and PowerShell and their forms in Nurt |
| [Specification](../../spec/catalog.md) | Language, catalog, report, journal and configuration |
| [Compatibility](compatibility.md) | Names and the Danaco Nurt Compatible program |
| [Versioning and support](versioning.md) | Versions, deprecation and supported systems |
| [Security](security.md) | Reporting vulnerabilities and the security model |

## Commands

The catalog has 148 commands. The 33 core commands cover files, text, records, programs, the journal and help. The other commands belong to 14 optional groups such as `git`, `code`, `archive` and `network`.
