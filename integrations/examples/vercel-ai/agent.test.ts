// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Tests of the example with the mock language model of the AI SDK; no network access.

import assert from "node:assert/strict";
import { test } from "node:test";

import { MockLanguageModelV2 } from "ai/test";

import { runAgent } from "./agent.ts";

const PROGRAM = 'find "/home/user/project" glob: "**/*.py" |> count';
const DEFINITION = {
  name: "nurt",
  description: "Runs programs of the Nurt language.",
  input_schema: {
    type: "object",
    properties: { program: { type: "string" }, limit: { type: "integer" } },
    additionalProperties: false,
  },
};
const USAGE = { inputTokens: 1, outputTokens: 1, totalTokens: 2 };

test("runs the tool call and returns the answer", async () => {
  const calls: Array<[Record<string, unknown>, Record<string, unknown>]> = [];
  const nurt = {
    run: async (input: Record<string, unknown>, options: Record<string, unknown>) => {
      calls.push([input, options]);
      return { text: "42 files", report: {} };
    },
  };
  const steps = [
    {
      content: [{ type: "tool-call" as const, toolCallId: "t1", toolName: "nurt", input: JSON.stringify({ program: PROGRAM, limit: 5 }) }],
      finishReason: "tool-calls" as const,
      usage: USAGE,
      warnings: [],
    },
    { content: [{ type: "text" as const, text: "There are 42 files." }], finishReason: "stop" as const, usage: USAGE, warnings: [] },
  ];
  const model = new MockLanguageModelV2({ doGenerate: async () => steps.shift()! });
  assert.equal(await runAgent(model, nurt, DEFINITION, "count files"), "There are 42 files.");
  assert.deepEqual(calls, [[{ program: PROGRAM }, { limit: 5 }]]);
});
