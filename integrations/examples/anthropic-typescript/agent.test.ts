// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           integrations/examples/anthropic-typescript/agent.test.ts
// Responsibility:   Source file integrations/examples/anthropic-typescript/agent.test.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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
  const client = { messages: { create: async (request: Record<string, unknown>) => (requests.push(structuredClone(request)), responses.shift()!) } };
  const nurt = new MockNurt();
  assert.equal(await runAgent(client, nurt, { name: "nurt" }, "count files"), "There are 42 files.");
  assert.deepEqual(nurt.calls, [[{ program: PROGRAM }, { limit: 5 }]]);
  const messages = requests[1]!["messages"] as Array<{ content: unknown }>;
  assert.deepEqual(messages.at(-1)!.content, [{ type: "tool_result", tool_use_id: "t1", content: "42 files", is_error: false }]);
});

test("marks a failed run as a tool error and falls back to the report", async () => {
  const failing = { async run() { return { ok: false, report: { status: "failed" } }; } };
  assert.deepEqual(await callNurt(failing, { program: PROGRAM }), ['{"status":"failed"}', true]);
});

test("ignores the tool calls of other tools", async () => {
  const responses = [
    { content: [{ type: "tool_use", id: "t1", name: "bash", input: { command: "ls" } }, { type: "text", text: "done" }] },
  ];
  const client = { messages: { create: async () => responses.shift()! } };
  const nurt = new MockNurt();
  assert.equal(await runAgent(client, nurt, { name: "nurt" }, "list"), "done");
  assert.deepEqual(nurt.calls, []);
});

test("stops after the turn limit", async () => {
  let requests = 0;
  const client = {
    messages: {
      create: async () => (requests += 1, { content: [{ type: "tool_use", id: "t", name: "nurt", input: { program: PROGRAM } }] }),
    },
  };
  await assert.rejects(runAgent(client, new MockNurt(), { name: "nurt" }, "loop"), /no answer after 20 turns/);
  assert.equal(requests, 20);
});

test("drops null arguments", async () => {
  const nurt = new MockNurt();
  await callNurt(nurt, { cursor: "c-2", program: null, limit: null });
  assert.deepEqual(nurt.calls, [[{ cursor: "c-2" }, {}]]);
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
