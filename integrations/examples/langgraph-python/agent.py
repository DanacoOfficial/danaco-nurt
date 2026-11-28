# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           integrations/examples/langgraph-python/agent.py
# Responsibility:   Script agent.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""LangChain agent (LangGraph runtime) with the nurt tool.

Usage: python agent.py "count the Python files under /home/user/project"
"""

import json
import os
import subprocess
import sys

MODEL = os.environ.get("LANGCHAIN_MODEL", "anthropic:claude-sonnet-4-5")


def tool_definition(tool_format):
    """Return the definition of the nurt tool rendered by `nurt schema tool`."""
    result = subprocess.run(
        ["nurt", "schema", "tool", "--format", tool_format],
        capture_output=True,
        check=True,
        text=True,
    )
    return json.loads(result.stdout)


def nurt_tool(nurt, definition):
    """Build a LangChain tool from the neutral definition of the nurt tool."""
    from langchain_core.tools import StructuredTool

    def run(**arguments):
        try:
            response = nurt.run(**arguments)
        except TypeError as error:
            return f"error: {error}"
        return response.text

    return StructuredTool.from_function(
        func=run,
        name=definition["name"],
        description=definition["description"],
        args_schema=definition["input_schema"],
    )


def build_agent(model, tool):
    """Create the agent graph with the nurt tool."""
    from langchain.agents import create_agent

    return create_agent(model, tools=[tool])


def run_agent(agent, task):
    """Run the agent and return the text of the last message."""
    state = agent.invoke({"messages": [{"role": "user", "content": task}]})
    return state["messages"][-1].content


def main():
    from nurt import Nurt

    with Nurt() as nurt:
        tool = nurt_tool(nurt, tool_definition("json-schema"))
        print(run_agent(build_agent(MODEL, tool), " ".join(sys.argv[1:])))


if __name__ == "__main__":
    main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
