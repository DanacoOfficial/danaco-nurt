<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
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
| `nurt serve --socket <path>` | as above, over a Unix domain socket or a named pipe | a console shared by several host processes | as above | as above |
| `libnurt` (`nurt.h`) | C ABI in the host process | short calls without effects: `nurt_check`, `nurt_describe`, `nurt_correct`; hosts without child processes | callback `nurt_event_cb` | none; a fault in the library stops the host |
| `nurt mcp --stdio` | Model Context Protocol over standard input and output | MCP clients: desktop and editor assistants, frameworks with MCP support | MCP notifications | separate process |
| `nurt mcp --http <address>` | Model Context Protocol, streamable HTTP | a server shared over the network | MCP notifications | separate process |

Selection:

1. A host that runs its own agent loop uses `nurt serve --stdio`. It supervises the child process: it restarts the process after an exit and fails the requests in progress with a call error. The journal is on disk, so a restarted console recovers completed operations.
2. A host that only analyses programs (policy, highlighting, correction) can call `libnurt` in process.
3. A host that already speaks MCP starts `nurt mcp`; no code is needed.

The language bindings (`bindings/`) wrap `nurt serve` and `libnurt` with the same API in Python, TypeScript, Go, Java and .NET.

Every interface accepts the request envelope `nurt-request/1` and returns the response `nurt-response/1` with the effect report `nurt-report/1`. Errors of the language (an unknown command, a failed assertion) are part of the report; call errors (malformed JSON, an unknown method, a stopped process) are errors of the interface.
