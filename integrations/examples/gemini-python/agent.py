# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           integrations/examples/gemini-python/agent.py
# Responsibility:   Script agent.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Agent loop with the Google Gen AI SDK (Gemini API) and the nurt tool.

Usage: python agent.py "count the Python files under /home/user/project"
"""

import json
import os
import subprocess
import sys

MODEL = os.environ.get("GEMINI_MODEL", "gemini-2.5-flash")
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
    """Run one tool call and return the function response.

    The `gemini` format passes `ast` as JSON text; the binding parses it.
    """
    try:
        response = nurt.run(**dict(arguments))
    except TypeError as error:
        return {"error": str(error)}
    if response.ok:
        return {"output": response.text}
    return {"error": response.text}


def run_agent(client, nurt, declaration, task, model=MODEL):
    """Run the agent loop until the model answers without a function call."""
    config = {
        "tools": [{"function_declarations": [declaration]}],
        "automatic_function_calling": {"disable": True},
    }
    contents = [{"role": "user", "parts": [{"text": task}]}]
    for _ in range(MAX_TURNS):
        response = client.models.generate_content(model=model, contents=contents, config=config)
        calls = response.function_calls or []
        if not calls:
            return response.text
        contents.append(response.candidates[0].content)
        parts = []
        for call in calls:
            result = call_nurt(nurt, call.args) if call.name == "nurt" else {"error": "unknown tool"}
            parts.append({"function_response": {"id": call.id, "name": call.name, "response": result}})
        contents.append({"role": "user", "parts": parts})
    raise RuntimeError(f"no answer after {MAX_TURNS} turns")


def main():
    from google import genai
    from nurt import Nurt

    client = genai.Client()
    with Nurt() as nurt:
        print(run_agent(client, nurt, tool_definition("gemini"), " ".join(sys.argv[1:])))


if __name__ == "__main__":
    main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
