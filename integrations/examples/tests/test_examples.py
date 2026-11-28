# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           integrations/examples/tests/test_examples.py
# Responsibility:   Script test_examples.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Tests of the Python examples with a mock model and a mock console.

The mock clients reproduce the response objects of each SDK that the
examples read; the mock console records the calls instead of running
programs. The tests need neither network access nor API keys. The tests of
the LangChain and Semantic Kernel examples run when those packages are
installed.
"""

import asyncio
import importlib.util
import json
import pathlib
import re
import unittest
from types import SimpleNamespace

EXAMPLES = pathlib.Path(__file__).resolve().parent.parent
PROGRAM = 'find "/home/user/project" glob: "**/*.py" |> count'
TREE = {"nurt": "1.0", "program": {"statements": []}}


def load(directory):
    """Load `agent.py` of an example directory as a module."""
    spec = importlib.util.spec_from_file_location(
        directory.replace("-", "_"), EXAMPLES / directory / "agent.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def installed(name):
    try:
        return importlib.util.find_spec(name) is not None
    except ModuleNotFoundError:
        return False


class MockNurt:
    """Console that records the arguments of every run."""

    def __init__(self, text="42 files", ok=True):
        self.calls = []
        self.text = text
        self.ok = ok

    def run(self, program=None, *, ast=None, **options):
        allowed = {"preview", "limit", "format", "cursor", "accept", "base"}
        unknown = set(options) - allowed
        if unknown:
            raise TypeError(f"unknown option {sorted(unknown)[0]!r}")
        if isinstance(ast, str):
            ast = json.loads(ast)
        options = {key: value for key, value in options.items() if value is not None}
        self.calls.append({"program": program, "ast": ast, **options})
        return SimpleNamespace(text=self.text, ok=self.ok)


class Scripted:
    """Callable that returns the scripted responses in order and records the requests."""

    def __init__(self, *responses):
        self.responses = list(responses)
        self.requests = []

    def __call__(self, **request):
        # The loops extend the same message list after the call; keep the state at the call.
        self.requests.append({key: list(value) if isinstance(value, list) else value for key, value in request.items()})
        return self.responses.pop(0)


def block(kind, **fields):
    return SimpleNamespace(type=kind, **fields)


class AnthropicExampleTest(unittest.TestCase):
    def test_runs_the_tool_call_and_returns_the_answer(self):
        agent = load("anthropic-python")
        create = Scripted(
            SimpleNamespace(content=[block("tool_use", id="t1", name="nurt", input={"program": PROGRAM})]),
            SimpleNamespace(content=[block("text", text="There are 42 files.")]),
        )
        client = SimpleNamespace(messages=SimpleNamespace(create=create))
        nurt = MockNurt()
        answer = agent.run_agent(client, nurt, {"name": "nurt"}, "count files")
        self.assertEqual(answer, "There are 42 files.")
        self.assertEqual(nurt.calls, [{"program": PROGRAM, "ast": None}])
        result = create.requests[1]["messages"][-1]["content"][0]
        self.assertEqual(result, {"type": "tool_result", "tool_use_id": "t1", "content": "42 files", "is_error": False})

    def test_reports_unknown_argument_as_tool_error(self):
        agent = load("anthropic-python")
        self.assertEqual(agent.call_nurt(MockNurt(), {"program": PROGRAM, "command": "ls"}), ("unknown option 'command'", True))
        self.assertEqual(agent.call_nurt(MockNurt(ok=False), {"program": PROGRAM}), ("42 files", True))

    def test_ignores_tool_calls_of_other_tools(self):
        agent = load("anthropic-python")
        create = Scripted(
            SimpleNamespace(content=[block("tool_use", id="t1", name="bash", input={"command": "ls"}), block("text", text="done")])
        )
        client = SimpleNamespace(messages=SimpleNamespace(create=create))
        nurt = MockNurt()
        self.assertEqual(agent.run_agent(client, nurt, {"name": "nurt"}, "list"), "done")
        self.assertEqual(nurt.calls, [])

    def test_stops_after_the_turn_limit(self):
        agent = load("anthropic-python")
        turns = [
            SimpleNamespace(content=[block("tool_use", id=f"t{index}", name="nurt", input={"program": PROGRAM})])
            for index in range(agent.MAX_TURNS)
        ]
        create = Scripted(*turns)
        client = SimpleNamespace(messages=SimpleNamespace(create=create))
        with self.assertRaisesRegex(RuntimeError, "no answer after 20 turns"):
            agent.run_agent(client, MockNurt(), {"name": "nurt"}, "loop")
        self.assertEqual(len(create.requests), agent.MAX_TURNS)


class OpenAIExampleTest(unittest.TestCase):
    def test_strict_arguments_with_null_reach_the_console_without_null(self):
        agent = load("openai-python")
        arguments = json.dumps({"program": PROGRAM, "cursor": None, "limit": 20, "ast": None})
        call = SimpleNamespace(id="c1", function=SimpleNamespace(name="nurt", arguments=arguments))
        create = Scripted(
            SimpleNamespace(choices=[SimpleNamespace(message=SimpleNamespace(content=None, tool_calls=[call]))]),
            SimpleNamespace(choices=[SimpleNamespace(message=SimpleNamespace(content="42", tool_calls=None))]),
        )
        client = SimpleNamespace(chat=SimpleNamespace(completions=SimpleNamespace(create=create)))
        nurt = MockNurt()
        self.assertEqual(agent.run_agent(client, nurt, {"type": "function"}, "count files"), "42")
        self.assertEqual(nurt.calls, [{"program": PROGRAM, "ast": None, "limit": 20}])
        self.assertEqual(create.requests[1]["messages"][-1], {"role": "tool", "tool_call_id": "c1", "content": "42 files"})

    def test_reports_malformed_arguments_to_the_model(self):
        agent = load("openai-python")
        self.assertTrue(agent.call_nurt(MockNurt(), "{").startswith("error: "))


class GeminiExampleTest(unittest.TestCase):
    def test_parses_the_tree_passed_as_json_text(self):
        agent = load("gemini-python")
        call = SimpleNamespace(id="f1", name="nurt", args={"ast": json.dumps(TREE), "preview": True})
        model_content = {"role": "model", "parts": [{"function_call": {"name": "nurt"}}]}
        generate = Scripted(
            SimpleNamespace(function_calls=[call], candidates=[SimpleNamespace(content=model_content)], text=None),
            SimpleNamespace(function_calls=None, candidates=[], text="Preview done."),
        )
        client = SimpleNamespace(models=SimpleNamespace(generate_content=generate))
        nurt = MockNurt()
        self.assertEqual(agent.run_agent(client, nurt, {"name": "nurt"}, "preview"), "Preview done.")
        self.assertEqual(nurt.calls, [{"program": None, "ast": TREE, "preview": True}])
        part = generate.requests[1]["contents"][-1]["parts"][0]["function_response"]
        self.assertEqual(part, {"id": "f1", "name": "nurt", "response": {"output": "42 files"}})
        self.assertEqual(agent.call_nurt(MockNurt(ok=False), {"program": PROGRAM}), {"error": "42 files"})


@unittest.skipUnless(installed("langchain_core"), "langchain-core is not installed")
class LangGraphExampleTest(unittest.TestCase):
    def test_tool_runs_the_console_with_the_arguments_of_the_call(self):
        agent = load("langgraph-python")
        definition = {
            "name": "nurt",
            "description": "Runs programs of the Nurt language.",
            "input_schema": {
                "type": "object",
                "properties": {"program": {"type": "string"}, "limit": {"type": "integer"}},
                "additionalProperties": False,
            },
        }
        nurt = MockNurt()
        tool = agent.nurt_tool(nurt, definition)
        self.assertEqual(tool.name, "nurt")
        self.assertEqual(tool.invoke({"program": PROGRAM, "limit": 5}), "42 files")
        self.assertEqual(nurt.calls, [{"program": PROGRAM, "ast": None, "limit": 5}])


@unittest.skipUnless(installed("semantic_kernel"), "semantic-kernel is not installed")
class SemanticKernelExampleTest(unittest.TestCase):
    def test_plugin_starts_the_mcp_server_of_the_console(self):
        agent = load("semantic-kernel-python")
        plugin = agent.nurt_plugin()
        self.assertEqual(plugin.name, "nurt")
        self.assertEqual(plugin.args, ["mcp", "--stdio"])

    def test_returns_the_text_of_the_answer(self):
        agent = load("semantic-kernel-python")

        class MockAgent:
            async def get_response(self, messages):
                return SimpleNamespace(content=f"answer to {messages}")

        self.assertEqual(asyncio.run(agent.run_agent(MockAgent(), "task")), "answer to task")


class SweAgentToolTest(unittest.TestCase):
    """The tool of SWE-agent hands the whole program over as one argument.

    SWE-agent quotes the value of an argument for the shell
    (``shlex.quote`` in its function calling parser) before it replaces the
    placeholder of the signature, so a program with spaces and double quotes
    reaches ``nurt`` as a single argument. The signature must therefore hold
    exactly one placeholder and the argument must be a required string.
    """

    def setUp(self):
        self.text = (EXAMPLES / "swe-agent" / "tools" / "nurt" / "config.yaml").read_text(encoding="utf-8")

    def test_signature_has_the_program_as_its_only_placeholder(self):
        match = re.search(r'(?m)^\s*signature:\s*"([^"]+)"', self.text)
        self.assertIsNotNone(match)
        self.assertEqual(match.group(1), "nurt <program>")

    def test_program_is_a_required_string_argument(self):
        block = self.text[self.text.index("arguments:") :]
        self.assertRegex(block, r"name: program\s+type: string")
        self.assertRegex(block, r"required: true")
        self.assertNotIn("argument_format", block)


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
