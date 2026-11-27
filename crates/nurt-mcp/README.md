<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           crates/nurt-mcp/README.md -->
<!-- Responsibility:   Documents nurt-mcp. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# nurt-mcp

Model Context Protocol (MCP) server of Danaco Nurt, started by `nurt mcp`.

The server lets any MCP client, such as the host of an AI agent, run programs of the Nurt language without a dedicated integration.

## Transports

| Transport | Command | Build feature |
|---|---|---|
| standard input and output (default) | `nurt mcp` | always |
| streamable HTTP | `nurt mcp --http <address>` | `http` |

In the stdio transport, standard output carries only protocol messages, one JSON-RPC message per line. Diagnostics are written to standard error.

## Protocol versions

The server negotiates the protocol versions `2024-11-05`, `2025-03-26`, `2025-06-18` and `2025-11-25`. A client that asks for another version in `initialize` receives `2025-11-25` and decides whether to continue. A request that declares an unsupported version in its metadata is rejected with error `-32022`.

## Tools

| Tool | Arguments | Result |
|---|---|---|
| `nurt` | the request `nurt-request/1`: one of `program`, `ast`, `cursor`, `accept`, and the call options (`preview`, `format`, `limit`, `session`, ...) at the top level | `content[0]`: the report rendered in `format` (default `text`); `structuredContent`: the report `nurt-report/1` |
| `nurt_help` | `topic`, `kind`, `examples`, `language` | built-in help as text |
| `nurt_check` | one of `program` and `accept`; optional `session` | `content[0]` and `structuredContent`: the document `nurt-check/1` with the member `bound`; `isError: true` with `structuredContent.error` when the program is rejected |

The definition of `nurt` is the `mcp` format of `nurt schema tool`. A result has `isError: true` only when the report carries an error or the call itself failed. The tool `nurt` is annotated `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: true`; `nurt_help` and `nurt_check` are read-only; `nurt_check` binds and plans the program and does not run it.

## Resources

| URI | Media type | Content |
|---|---|---|
| `nurt://help/{topic}` | `text/plain` | built-in help of a command, group, error code, `syntax` or `corrections` |
| `nurt://schema/{name}` | `application/schema+json` | schema `ast`, `bound`, `report`, `request`, `config` or `tool` |
| `nurt://describe/core` | `text/plain` | core description layer |
| `nurt://describe/group/{name}` | `text/plain` | description layer of a group of commands |

`resources/templates/list` returns the templates with parameters.

## Progress, cancellation and logging

| Client message | Effect |
|---|---|
| `_meta.progressToken` in `tools/call` | progress and output of launched programs are sent as `notifications/progress` |
| `notifications/cancelled` | the running program is cancelled |
| `logging/setLevel` | log messages of the engine at or above the level are sent as `notifications/message`; default level `warning` |

Calls run concurrently up to the limit `max_inflight` of the engine. A call beyond the limit is rejected with error `-32000`.

## Client configuration

Clients that read an `mcpServers` object start the server as a child process:

```json
{
  "mcpServers": {
    "nurt": {
      "command": "nurt",
      "args": ["mcp"]
    }
  }
}
```

On Windows, give the full path of the program, for example `C:/Users/user/.nurt/bin/nurt.exe`.

A client that connects to a running HTTP server uses the endpoint `/mcp`:

```json
{
  "mcpServers": {
    "nurt": {
      "url": "http://127.0.0.1:8765/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

## HTTP transport

The transport follows the streamable HTTP transport of MCP, revision `2025-03-26` and later.

| Property | Behavior |
|---|---|
| endpoint | `POST /mcp` sends a message; `GET /mcp` opens the event stream of the session; `DELETE /mcp` ends the session |
| address | `127.0.0.1` by default; any other address requires an explicit option |
| session | `initialize` returns `Mcp-Session-Id`; the identifier is the default engine session of the calls of the session; a session without messages for one hour is closed; a request of an ended session receives `404` |
| response | a request is answered with `text/event-stream`: progress notifications of the call as they occur, then the response; notifications and responses of the client receive `202` |
| session stream | messages that belong to no request, such as log messages, are sent on the stream opened by `GET` |
| resumption | every event carries an identifier; `GET` with `Last-Event-ID` sends the events of the interrupted stream again |
| `Accept` | `POST` requires `application/json` and `text/event-stream`, `GET` requires `text/event-stream`; otherwise `406` |
| `Host` | loopback hosts, the listening address and hosts given in the options are accepted; other hosts receive `403` |
| `Origin` | loopback origins and origins given in the options are accepted; other origins receive `403` |
| bearer token | optional, read from a file or an environment variable; without a token no `Authorization` header is required |

The server applies no data policy of its own: every program goes to the engine, which applies the configuration.

## Library use

```rust,no_run
use std::sync::Arc;

use nurt_mcp::{Backend, McpOptions, serve_stdio};

fn serve(engine: Arc<impl Backend>) -> Result<(), nurt_mcp::McpError> {
    serve_stdio(engine, McpOptions::new())
}
```

The engine is reached through the trait `Backend`. `serve_io` serves one client over any pair of byte streams.

## License

Apache-2.0. See `LICENSE` at the root of the repository.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
