<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/integrators/README.md -->
<!-- Responsibility:   Documents Integrator guide. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Integrator guide

Reference for hosts that give an AI agent the `nurt` tool: agent runtimes, SDK integrations, editors and services that run Danaco Nurt on behalf of a model.

| Page | Content |
|---|---|
| [Interfaces](#interfaces) | `nurt serve`, the C ABI and the MCP server |
| [Tool definition](tool-definition.md) | `nurt schema tool`, formats, rules per platform, command groups loaded on demand |
| [Tool calls](tool-calls.md) | arguments and requests, corrections and `accept`, pages and cursors |
| [Policy](policy.md) | host policy with `check` and `preview`: forbidden commands, allowed directories, HTTP targets |
| [Remote instances and events](remote-and-events.md) | sending syntax trees to another device, events of background jobs |
| [Versions and compatibility](versioning.md) | format versions, protocol negotiation, compatibility rules |

Examples for model SDKs and agent frameworks: `integrations/examples/`.

## Interfaces

| Interface | Transport | Use | Events | Isolation |
|---|---|---|---|---|
| `nurt serve --stdio` | JSON-RPC 2.0 (`nurt-rpc/1`) over standard input and output of a child process | agent runtimes and services; one process per session or workspace | notifications `nurt/event` in the same stream | separate process; a crash of the console does not stop the host |
| `nurt serve --socket <path>` | reserved for a transport over a Unix domain socket or a named pipe; the option is recognised and ends with exit code 3 in every build of release 0.1.2 | none; connect with `nurt serve --stdio` | none | none |
| `libnurt` (`nurt.h`) | C ABI in the host process | short calls without effects: `nurt_check`, `nurt_describe`, `nurt_correct`; hosts without child processes | callback `nurt_event_cb` | none; a fault in the library stops the host |
| `nurt mcp --stdio` | Model Context Protocol over standard input and output | MCP clients: desktop and editor assistants, frameworks with MCP support | MCP notifications | separate process |
| `nurt mcp --http <address>` | Model Context Protocol, streamable HTTP | a server shared over the network | MCP notifications | separate process |

The `nurt mcp` subcommand is present in builds that link the feature `mcp`; a build without it reports the subcommand as unavailable. `nurt serve` is present in every build. Framing, methods, lifecycle, events and error codes of `nurt serve` are defined in [`docs/reference/serve-protocol.md`](../reference/serve-protocol.md); this guide does not repeat them.

Selection:

1. A host that runs its own agent loop uses `nurt serve --stdio`. It supervises the child process: it restarts the process after an exit and fails the requests in progress with a call error. The journal is on disk, so a restarted console recovers completed operations.
2. A host that only analyses programs (policy, highlighting, correction) can call `libnurt` in process.
3. A host that already speaks MCP starts `nurt mcp`; no code is needed.

The language bindings (`bindings/`) wrap `nurt serve` and `libnurt` with the same API in Python, TypeScript, Go, Java and .NET.

Every interface accepts the request envelope `nurt-request/1` and returns the response `nurt-response/1` with the effect report `nurt-report/1`. Errors of the language (an unknown command, a failed assertion) are part of the report; call errors (malformed JSON, an unknown method, a stopped process) are errors of the interface.

## Calls to `nurt serve`

A request is a JSON-RPC 2.0 message of `nurt-rpc/1`; `check` takes the same parameters as `run`. Methods, parameters and results are listed in the [protocol reference](../reference/serve-protocol.md):

<!-- schema: rpc -->
```json
{"jsonrpc": "2.0", "id": 1, "method": "check", "params": {"nurt": "1.0", "program": "list \"/home/user/app\""}}
```

A call error replaces the result with the `error` object; the codes and their mapping to the statuses of the C ABI (`-32000` minus the status) are defined in the protocol reference:

<!-- schema: rpc -->
```json
{"jsonrpc": "2.0", "id": 2, "error": {"code": -32002, "message": "the call was cancelled", "data": {"status": "NURT_E_CANCELLED"}}}
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
