// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Tests of the example with a mock model and a mock console; no network access.

import assert from "node:assert/strict";
import { test } from "node:test";

import { callNurt, runAgent } from "./agent.ts";

const PROGRAM = 'find "/home/user/project" glob: "**/*.py" |> count';

class MockNurt {
  readonly calls: Array<[Record<string, unknown>, Record<string, unknown>]> = [];
  async run(input: Record<string, unknown>, options: Record<string, unknown>) {
    this.calls.push([input, options]);
    return { text: "42 files", ok: true, report: {} };
  }
}

test("runs the tool call and returns the answer", async () => {
  const requests: Array<Record<string, unknown>> = [];
  const responses = [
    { content: [{ type: "tool_use", id: "t1", name: "nurt", input: { program: PROGRAM, limit: 5 } }] },
    { content: [{ type: "text", text: "There are 42 files." }] },
  ];
  const client = { messages: { create: async (request: Record<string, unknown>) => (requests.push(request), responses.shift()!) } };
  const nurt = new MockNurt();
  assert.equal(await runAgent(client, nurt, { name: "nurt" }, "count files"), "There are 42 files.");
  assert.deepEqual(nurt.calls, [[{ program: PROGRAM }, { limit: 5 }]]);
  const messages = requests[1]!["messages"] as Array<{ content: unknown }>;
  assert.deepEqual(messages.at(-1)!.content, [{ type: "tool_result", tool_use_id: "t1", content: "42 files", is_error: false }]);
});

test("drops null arguments", async () => {
  const nurt = new MockNurt();
  await callNurt(nurt, { cursor: "c-2", program: null, limit: null });
  assert.deepEqual(nurt.calls, [[{ cursor: "c-2" }, {}]]);
});
