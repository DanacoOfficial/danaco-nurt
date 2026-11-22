# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Agent loop with the Anthropic Messages API and the nurt tool.

Usage: python agent.py "count the Python files under /home/user/project"
"""

import json
import os
import subprocess
import sys

MODEL = os.environ.get("ANTHROPIC_MODEL", "claude-sonnet-4-5")
MAX_TOKENS = 4096
MAX_TURNS = 20


def tool_definition(tool_format):
    """Return the definition of the nurt tool rendered by `nurt schema tool`."""
    result = subprocess.run(
        ["nurt", "schema", "tool", "--format", tool_format],
        capture_output=True,
        check=True,
        text=True,
    )
    return json.loads(result.stdout)


def call_nurt(nurt, arguments):
    """Run one tool call; return the text for the model and whether it is an error."""
    try:
        response = nurt.run(**arguments)
    except TypeError as error:
        return str(error), True
    return response.text, not response.ok


def run_agent(client, nurt, tool, task, model=MODEL):
    """Run the agent loop until the model answers without a tool call."""
    messages = [{"role": "user", "content": task}]
    for _ in range(MAX_TURNS):
        response = client.messages.create(
            model=model, max_tokens=MAX_TOKENS, tools=[tool], messages=messages
        )
        messages.append({"role": "assistant", "content": response.content})
        results = []
        for block in response.content:
            if block.type == "tool_use" and block.name == "nurt":
                text, is_error = call_nurt(nurt, block.input)
                results.append(
                    {
                        "type": "tool_result",
                        "tool_use_id": block.id,
                        "content": text,
                        "is_error": is_error,
                    }
                )
        if not results:
            return "".join(block.text for block in response.content if block.type == "text")
        messages.append({"role": "user", "content": results})
    raise RuntimeError(f"no answer after {MAX_TURNS} turns")


def main():
    import anthropic
    from nurt import Nurt

    client = anthropic.Anthropic()
    with Nurt() as nurt:
        print(run_agent(client, nurt, tool_definition("anthropic"), " ".join(sys.argv[1:])))


if __name__ == "__main__":
    main()
