---
command: code.languages
group: code
summary: List supported languages with file extensions, grammar versions and language server state.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.languages

## Synopsis

```text
code.languages
```

## Description

Lists one `language` record for every language with a built-in grammar, in
the order of the grammar registry, followed by the languages that have only
a configured language server, in alphabetical order.

The server fields come from the pool of language servers. The command does
not start servers and does not wait for requests in progress, so it answers
while other commands use the servers. A server is reported as found when its
program is on `PATH` and `code.servers_allow` allows it. Without a server
pool, `server` gives the configured program and `server_found` is `false`.

## Parameters

The command has no parameters.

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `language` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

List every language.

```nurt example
code.languages
```

List the languages whose server is installed.

```nurt example
code.languages |> filter server_found
```

Show the running servers with their roots.

```nurt example
code.languages |> filter server_running |> select language, server, server_pid, server_root
```

## Errors

The command reports no errors of its own.

## Related

`code.diagnostics`, `code.rename`, configuration keys `code.servers`,
`code.servers_allow`, `code.max_servers`.
