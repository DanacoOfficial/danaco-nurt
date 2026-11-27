<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           integrations/examples/mcp/README.md -->
<!-- Responsibility:   Documents Model Context Protocol clients. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Model Context Protocol clients

Client configurations for the MCP server `nurt mcp`, which provides the `nurt` tool.

| File | Client |
|---|---|
| `mcp-servers.json` | clients that read an `mcpServers` object (desktop and command-line clients) |
| `vscode-mcp.json` | Visual Studio Code, as `.vscode/mcp.json` of a workspace |
| `streamable-http.json` | clients that connect to a running server over streamable HTTP |

The `stdio` configurations start the server as a child process of the client. For HTTP, start the server first:

```sh
nurt mcp --http 127.0.0.1:8765
```

The definition of the tool listed by the server equals `nurt schema tool --format mcp`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
