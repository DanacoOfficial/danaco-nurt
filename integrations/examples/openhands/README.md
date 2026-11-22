<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# OpenHands

Configuration that adds the `nurt` tool to OpenHands through the MCP server `nurt mcp`.

## Usage

1. Install the `nurt` executable in the runtime image of OpenHands, so that `nurt` is in `PATH` of the sandbox.
2. Merge the `[mcp]` section of `config.toml` into the configuration of the installation.
3. Start OpenHands; the agent lists the `nurt` tool next to its built-in tools.

The server runs inside the sandbox, so programs act on the workspace of the agent.
