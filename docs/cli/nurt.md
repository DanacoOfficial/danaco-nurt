<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# nurt

The `nurt` executable runs programs of the language, offers an interactive session and hosts the protocol servers.

## Invocation

```
nurt [OPTIONS] <program>
nurt <subcommand> [arguments]
nurt <subcommand> -h | --help
nurt -V | --version | -h | --help
```

The first argument selects a subcommand only when it equals the name of a subcommand exactly. Any other first argument starts a program of the language, and all arguments are joined with single spaces into the program text. `nurt exec` is always unambiguous. Names are case-sensitive; an argument in quotes or with surrounding white space is a program.

Without arguments the executable starts the interactive session.

| Option | Type | Default | Meaning |
|---|---|---|---|
| `-V` | flag | | Print `nurt <version>`. |
| `--version` | flag | | Print the version with the build data, the copyright and the license. |
| `-h`, `--help` | flag | | Print the usage. As the first argument after the name of a subcommand it prints the usage of that subcommand and ends with exit code 0, without running it. |

The options of a program (`--variant`, `--preview`, `--json`, `--format`, `--limit`, `--cursor`, `--verbosity`) are accepted before the first word of a program, so `nurt --preview remove ...` equals `nurt exec --preview remove ...`. Words after the first word of a program belong to the program. The language has no options with a dash.

## Output and colour

The result is written to the standard output in the requested format. Errors of the command line are written to the standard error output. A language error of `check`, `fmt`, `ast` and `schema` is written to the standard error output as text, except with an explicit `--format json` or `--json`: then it is a JSON document on the standard output (`{"error": {...}}`, and `{"diagnostics": [], "error": {...}}` for `check`, as in `docs/reference/c-abi.md`), the standard error output stays empty and the exit code is the one the error has in the text form. Errors that are not language errors, such as an invalid option or an unreadable file, are always written to the standard error output as text. Colours follow `output.color` (`NURT_COLOR`): with `auto`, only a terminal is coloured and the variable `NO_COLOR` disables colours. The banner of an interactive session is written only to an interactive terminal.

## Build features

The subcommands `mcp`, `lsp`, `migrate`, `service` and `journal` are implemented by optional components. Each component is linked by a Cargo feature of the `nurt-cli` crate; the default build links all of them (the default features of `nurt-cli`); a build with `--no-default-features` links none. A subcommand whose component is not part of the build writes `nurt: <subcommand> is not available in this build` to the standard error output and ends with exit code 3. `nurt doctor` lists these subcommands with the feature each one needs.

| Feature | Links | Subcommand |
|---|---|---|
| `mcp` | `nurt-mcp` | `mcp --stdio` |
| `mcp-http` | `nurt-mcp` with the HTTP transport | `mcp --http` |
| `lsp` | `nurt-langserver` | `lsp` |
| `migrate` | `nurt-migrate` | `migrate` |
| `service` | `nurt-service` | `service` |
| `journal` | `nurt-journal` | `journal salvage` |
| `stub` | `nurt-ffi` with recorded answers | `serve` (testing only) |

## `exec`

Runs a program given as arguments.

Syntax:

```
nurt exec [--variant a|b|c] [--preview] [--json | --format F] [--limit N]
          [--cursor C] [--verbosity V] <program>
```

| Parameter | Type | Default | Meaning |
|---|---|---|---|
| `--variant` | `a`, `b`, `c` | configured variant | Notation of the program text. |
| `--preview` | flag | off | Compute the effect without applying it. |
| `--json` | flag | off | Same as `--format json`. |
| `--format` | `text`, `json`, `markdown`, `compact` | `text` | Format of the result. |
| `--limit` | integer 1 to 5000 | 50 | Records per page. |
| `--cursor` | text | none | Read a further page of an earlier result; no program is given. |
| `--verbosity` | `quiet`, `normal`, `detailed` | `normal` | Level of detail. |

Result: the rendered result of the program; with `--json` the response document. Exit code: see `exit-codes.md`.

Errors: a language error is part of the result and sets the exit code; an invalid option is a usage error (2).

Example:

```
nurt exec --preview remove "/tmp/build/old.log"
```

## `script`

Runs a script file. Each pipeline of the file is its own operation. A first line starting with `#!` is ignored, so a file can start with `#!/usr/bin/env nurt`.

Syntax:

```
nurt script <file | -> [--preview] [--json | --format F] [--limit N] [--verbosity V]
```

The file `-` is the standard input. The options are those of `exec`.

Errors: an unreadable file ends with exit code 1.

Example:

```
nurt script clean.nurt --preview
```

## `repl`

Starts an interactive session.

Syntax:

```
nurt repl
```

On a terminal the session offers a history kept in the state directory, completion of command names and parameters from the command catalog, highlighting and a banner: the name of the product drawn with characters above the version, the copyright line and a hint. The cursor is a blinking underscore while a line is typed. The banner, the prompt and the highlighting of the line use the colours of the product: silver and gold of the mark and the cyan of its blue. A terminal that reports 24-bit colour (`COLORTERM=truecolor`, Windows Terminal) shows exactly these colours, any other terminal the nearest colours of the 256-colour palette; `NO_COLOR` and `output.color` turn them off. Input that is not a terminal is read line by line without the banner. A line is a program, or one of these session words:

| Word | Meaning |
|---|---|
| `help [topic]` | Show the introduction or a topic. |
| `bg <program>` | Run a program in the background as a numbered job. |
| `jobs` | List the jobs. |
| `fg [id]` | Wait for a job and show its result; the newest job without `id`. |
| `kill [id]` | Cancel a job; the newest job without `id`. |
| `exit`, `quit` | End the session. |

Ctrl-C while a program runs cancels it and returns to the prompt; the program reports `E_CANCELLED` and the session continues. Ctrl-C at the prompt discards the line; Ctrl-D ends the session. The exit code is that of the last program that ran.

Example:

```
printf 'find "/srv"\nexit\n' | nurt repl
```

## `check`

Parses, corrects and binds a program and reports what it would touch, without running it.

Syntax:

```
nurt check <file | -e program> [--variant a|b|c] [--json | --format F]
```

| Parameter | Type | Default | Meaning |
|---|---|---|---|
| `file` | path or `-` | none | Program file; `-` is the standard input. |
| `-e` | text | none | Program text given on the command line. |

Result: `ok`, the number of corrections and the summary of the check; with `--json`, the check document. Errors: a language error is written to the standard error output; with `--json` or `--format json` it is the document `{"diagnostics": [], "error": {...}}` on the standard output, with the same exit code. The check reports the errors of the program that the run would report at the binding of its stages, among them `E_INVALID_VALUE` and `E_TYPE` for a literal compared with a field of a known type (`filter size > "abc"`), at the position of the literal.

Example:

```
nurt check -e 'remove "/tmp/build/old.log"'
```

## `fmt`

Prints a program in canonical form.

Syntax:

```
nurt fmt <file | -> [--variant a|b|c] [--json | --format F]
```

Result: the program text in the given variant, by default the configured variant. Errors: a language error is written as described in "Output and colour"; the successful result is the program text in every format.

Example:

```
nurt fmt clean.nurt --variant b
```

## `ast`

Converts a syntax tree between notations.

Syntax:

```
nurt ast [--from a|b|c|json] [--to json|cbor|a|b|c] [--json | --format F] <input | ->
```

| Parameter | Type | Default | Meaning |
|---|---|---|---|
| `--from` | `a`, `b`, `c`, `json` | `json` | Notation of the input. |
| `--to` | `json`, `cbor`, `a`, `b`, `c` | `json` | Notation of the output. |
| `--json`, `--format` | flag, `text`, `json`, `markdown`, `compact` | `text` | With `json`, a language error is a JSON document. |
| `input` | text or `-` | none | The input text; `-` is the standard input. |

Result: the tree in the requested notation; `cbor` is binary. The format only selects the form of a language error, as described in "Output and colour"; the notation of a successful result is selected by `--to`.

Example:

```
nurt ast --from a --to json 'find "/srv"'
```

## `schema`

Prints a published JSON Schema, or the definition of the `nurt` tool for an AI platform.

Syntax:

```
nurt schema ast|bound|report|request|config|tool [--json | --format json|text]
nurt schema tool [--format <format>] [--language en|pl] [--groups core|all|<group>,<group>...]
```

`nurt schema tool` without options prints the published schema `nurt-tool/1`, the schema of the neutral definition. With at least one option other than `--json`, `--format json` or `--format text` it prints the definition of the tool, rendered from the description texts and the revision of the command catalog of the installed engine: `--format` selects the platform (`anthropic`, `openai`, `openai-strict`, `openai-responses`, `openai-responses-strict`, `gemini`, `mcp` or `json-schema`; default `json-schema`), `--language` the language of the description (default `en`) and `--groups` the command groups described besides the core group (default `core`). The formats, the arguments of the tool and the rules per platform are in [Tool definition](../integrators/tool-definition.md).

Errors: an unknown name, format, language or group is `E_UNKNOWN_VALUE` with the known values; an unknown option or a missing value is a usage error. With `--json` or `--format json` an unknown name is written as a JSON document on the standard output, as described in "Output and colour".

Examples:

```
nurt schema report
nurt schema tool --format anthropic
```

## `catalog`

Describes the command catalog, a group or a command.

Syntax:

```
nurt catalog [command | group | check-distance]
```

Without an argument the command prints the summary of the catalog. `check-distance` is available when the correction components are part of the build.

Example:

```
nurt catalog remove
```

## `help`

Shows the built-in help.

Syntax:

```
nurt help [topic]
```

Without a topic the command prints an introduction; the first line names the product and its version. A topic is a command, a group or `core`.

Example:

```
nurt help remove
```

## `history`

Lists recorded operations. The command is a shortcut for the program `history` with the given arguments.

Syntax:

```
nurt history [arguments of the history command]
```

The options of `exec` are accepted before the arguments.

Example:

```
nurt history limit: 5
```

## `undo`

Undoes recorded operations. The command is a shortcut for the program `undo` with the given arguments.

Syntax:

```
nurt undo [arguments of the undo command]
```

Example:

```
nurt undo
```

## `redo`

Redoes undone operations. The command is a shortcut for the program `redo` with the given arguments.

Syntax:

```
nurt redo [arguments of the redo command]
```

Example:

```
nurt redo
```

## `serve`

Serves the request protocol to a client process.

Syntax:

```
nurt serve --stdio | --socket <path> [--framing ndjson|content-length] [--stub <directory>]
```

| Parameter | Type | Default | Meaning |
|---|---|---|---|
| `--stdio` | flag | none | Serves one client on the standard input and output. |
| `--socket <path>` | path | none | Serves a client on a local socket. Ends with exit code 3. |
| `--framing` | `ndjson` or `content-length` | `ndjson` | Framing of the messages. |
| `--stub <directory>` | path | none | Answers requests from recorded cases instead of the engine. Needs the `stub` feature; without it the command ends with exit code 3. |

Exactly one attachment is required. Nothing is written to the standard output except protocol messages, and no banner is printed. The exit code is 0 after `shutdown` or at the end of the input and 1 when the input cannot be read or the output cannot be written. Invalid arguments end with exit code 2.

Example:

```
nurt serve --stdio
```

## `mcp`

Serves the tool interface of the Model Context Protocol.

Syntax:

```
nurt mcp [--stdio | --http <address>]
```

Without the component in the build the command ends with exit code 3.

Example:

```
nurt mcp --stdio
```

## `lsp`

Serves the language protocol for `.nurt` files.

Syntax:

```
nurt lsp
```

Without the component in the build the command ends with exit code 3.

Example:

```
nurt lsp
```

## `migrate`

Translates a shell script to a Nurt script.

Syntax:

```
nurt migrate <file.sh | file.ps1>
```

Without the component in the build the command ends with exit code 3.

Example:

```
nurt migrate deploy.sh
```

## `service`

Manages the background service.

Syntax:

```
nurt service install | uninstall | status | run
```

Without the component in the build the command ends with exit code 3.

Example:

```
nurt service status
```

## `journal`

Maintains the journal.

Syntax:

```
nurt journal salvage <volume>
```

`salvage` repairs entries marked corrupt on a volume. Without the component in the build the command ends with exit code 3.

Example:

```
nurt journal salvage /
```

## `ext`

Manages extensions.

Syntax:

```
nurt ext install | remove | list
```

Without the component in the build the command ends with exit code 3.

Example:

```
nurt ext list
```

## `doctor`

Checks the installation and prints a table of checks: version, platform, formats, features, state directory (created and removed probe file), configuration files, services, journal, language components and the subcommands that are not available. A part that the build cannot examine is listed as `unavailable`. The exit code is 1 when a check failed, otherwise 0.

Syntax:

```
nurt doctor
```

Example:

```
nurt doctor
```

## `version`

Prints the full version, the same text as `nurt --version`.

Syntax:

```
nurt version
```

Result: three lines. The first is `nurt <version> (<commit> <date>)`; a build without repository data omits the parenthesis. The second is the copyright line, the third `License: Apache-2.0`. Exit code 0.

Example:

```
nurt version
```
