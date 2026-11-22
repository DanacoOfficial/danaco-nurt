# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Agent loop with the OpenAI Chat Completions API and the nurt tool.

Usage: python agent.py "count the Python files under /home/user/project"
"""

import json
import os
import subprocess
import sys

MODEL = os.environ.get("OPENAI_MODEL", "gpt-5")
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
    """Run one tool call and return the text for the model.

    The binding drops `null` arguments, which the strict format uses for
    absent ones, and accepts `ast` as JSON text.
    """
    try:
        response = nurt.run(**json.loads(arguments))
    except (TypeError, ValueError) as error:
        return f"error: {error}"
    return response.text


def run_agent(client, nurt, tool, task, model=MODEL):
    """Run the agent loop until the model answers without a tool call."""
    messages = [{"role": "user", "content": task}]
    for _ in range(MAX_TURNS):
        completion = client.chat.completions.create(model=model, messages=messages, tools=[tool])
        message = completion.choices[0].message
        messages.append(message)
        if not message.tool_calls:
            return message.content
        for call in message.tool_calls:
            if call.function.name == "nurt":
                content = call_nurt(nurt, call.function.arguments)
            else:
                content = f"error: unknown tool {call.function.name}"
            messages.append({"role": "tool", "tool_call_id": call.id, "content": content})
    raise RuntimeError(f"no answer after {MAX_TURNS} turns")


def main():
    import openai
    from nurt import Nurt

    client = openai.OpenAI()
    tool_format = os.environ.get("NURT_TOOL_FORMAT", "openai-strict")
    with Nurt() as nurt:
        print(run_agent(client, nurt, tool_definition(tool_format), " ".join(sys.argv[1:])))


if __name__ == "__main__":
    main()
