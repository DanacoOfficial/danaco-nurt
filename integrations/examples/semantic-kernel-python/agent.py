# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           integrations/examples/semantic-kernel-python/agent.py
# Responsibility:   Script agent.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Semantic Kernel agent with the nurt tool through the Model Context Protocol.

The kernel starts `nurt mcp` as a plugin and offers its `nurt` tool to the
chat model. Usage: python agent.py "count the Python files under /home/user/project"
"""

import asyncio
import os
import sys

MODEL = os.environ.get("OPENAI_MODEL", "gpt-5")
INSTRUCTIONS = "Use the nurt tool for every file, text, process and Git operation."


def nurt_plugin():
    """Return the MCP plugin that runs `nurt mcp` over standard input and output."""
    from semantic_kernel.connectors.mcp import MCPStdioPlugin

    return MCPStdioPlugin(
        name="nurt",
        description="Command shell and language where every change can be undone",
        command=os.environ.get("NURT_EXECUTABLE", "nurt"),
        args=["mcp", "--stdio"],
    )


def build_agent(service, plugin):
    """Create a chat completion agent that may call the tools of the plugin."""
    from semantic_kernel.agents import ChatCompletionAgent

    return ChatCompletionAgent(
        service=service, name="console", instructions=INSTRUCTIONS, plugins=[plugin]
    )


async def run_agent(agent, task):
    """Return the text of the answer of the agent."""
    response = await agent.get_response(messages=task)
    return str(response.content)


async def main():
    from semantic_kernel.connectors.ai.open_ai import OpenAIChatCompletion

    async with nurt_plugin() as plugin:
        agent = build_agent(OpenAIChatCompletion(ai_model_id=MODEL), plugin)
        print(await run_agent(agent, " ".join(sys.argv[1:])))


if __name__ == "__main__":
    asyncio.run(main())

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
