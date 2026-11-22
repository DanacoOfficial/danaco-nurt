---
title: Command line
order: 5
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Command line

## Synopsis

```text
nurt <program>
nurt exec [--variant a|b|c] [--preview] [--json | --format F] [--limit N] [--cursor C] [--verbosity V] <program>
nurt script <file.nurt> [--preview]
nurt check <file.nurt | -e '<program>'>
nurt fmt <file.nurt> [--variant a|b|c]
nurt ast [--from a|b|c|json] [--to json|cbor|a|b|c] <input>
nurt schema ast|bound|report|request|config|tool
nurt catalog [command | group]
nurt help [topic]
nurt history|undo|redo [arguments]
nurt serve --stdio | --socket <path>
nurt mcp [--stdio | --http <address>]
nurt lsp
nurt migrate <file.sh | file.ps1>
nurt service install|uninstall|start|stop|status|run
nurt journal salvage <volume>
nurt ext install|remove|list
nurt doctor
nurt version
```

## Description

`nurt` runs programs of the Danaco Nurt language and serves the engine to other programs. A first argument that is not a subcommand is a program: `nurt run ["go", "test"] cwd: "/home/user/app"`. `nurt exec` is always unambiguous.

The options with `--` belong to the program `nurt`. The language has no options with a hyphen; the corrections translate such habits.

## Subcommands

| Subcommand | Function |
|---|---|
| `exec` | runs a program from the argument |
| `script` | runs the pipelines of a file one after another; each pipeline is a separate operation |
| `repl` | interactive session with history, completion of names and parameters, and job control |
| `check` | parses, corrects and binds a program, prints the canonical form, changes nothing |
| `fmt` | prints a file in canonical form |
| `ast` | converts a program between the text variants and the syntax tree formats |
| `schema` | prints a JSON schema |
| `catalog` | prints the catalog, a group or one command |
| `help` | built-in help |
| `history`, `undo`, `redo` | shortcuts for the journal commands |
| `serve` | JSON-RPC server `nurt-rpc/1` for an agent engine |
| `mcp` | Model Context Protocol server that provides the tool `nurt` |
| `lsp` | language server for `.nurt` files |
| `migrate` | translates a shell script into a `.nurt` file |
| `service` | manages the background service of the journal |
| `journal salvage` | repairs entries marked `corrupt` |
| `ext` | manages extensions |
| `doctor` | reports file system properties, permissions, the service, versions and language servers |
| `version` | prints the version |

## Environment

Variables with the prefix `NURT_` set configuration keys; see [Configuration](configuration.md). `NURT_CONFIG` names an additional configuration file.

## Files

The system, user and project configuration files and the state directory are listed in [Configuration](configuration.md).

## See also

`nurt-commands(7)`, `nurt-<command>(1)`
