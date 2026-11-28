// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           integrations/examples/openai-typescript/agent.test.ts
// Responsibility:   Source file integrations/examples/openai-typescript/agent.test.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// Tests of the example with a mock model and a mock console; no network access.

import assert from "node:assert/strict";
import { test } from "node:test";

import { callNurt, runAgent } from "./agent.ts";

const TREE = { nurt: "1.0", program: { statements: [] } };

class MockNurt {
  readonly calls: Array<[Record<string, unknown>, Record<string, unknown>]> = [];
  async run(input: Record<string, unknown>, options: Record<string, unknown>) {
    this.calls.push([input, options]);
    return { text: "previewed", report: {} };
  }
}

test("strict arguments reach the console without null and with the tree parsed", async () => {
  const requests: Array<Record<string, unknown>> = [];
  const args = { program: null, ast: JSON.stringify(TREE), cursor: null, accept: null, preview: true, limit: null };
  const responses = [
    { id: "r1", output: [{ type: "function_call", name: "nurt", call_id: "c1", arguments: JSON.stringify(args) }], output_text: "" },
    { id: "r2", output: [{ type: "message" }], output_text: "Preview done." },
  ];
  const client = { responses: { create: async (request: Record<string, unknown>) => (requests.push(request), responses.shift()!) } };
  const nurt = new MockNurt();
  assert.equal(await runAgent(client, nurt, { type: "function" }, "preview"), "Preview done.");
  assert.deepEqual(nurt.calls, [[{ ast: TREE }, { preview: true }]]);
  assert.equal(requests[1]!["previous_response_id"], "r1");
  assert.deepEqual(requests[1]!["input"], [{ type: "function_call_output", call_id: "c1", output: "previewed" }]);
});

test("answers an unknown tool with an error and does not run the console", async () => {
  const requests: Array<Record<string, unknown>> = [];
  const responses = [
    { id: "r1", output: [{ type: "function_call", name: "bash", call_id: "c1", arguments: "{}" }], output_text: "" },
    { id: "r2", output: [{ type: "message" }], output_text: "Done." },
  ];
  const client = { responses: { create: async (request: Record<string, unknown>) => (requests.push(request), responses.shift()!) } };
  const nurt = new MockNurt();
  assert.equal(await runAgent(client, nurt, { type: "function" }, "list"), "Done.");
  assert.deepEqual(nurt.calls, []);
  assert.deepEqual(requests[1]!["input"], [{ type: "function_call_output", call_id: "c1", output: "error: unknown tool bash" }]);
});

test("falls back to the report when the console gives no text", async () => {
  const silent = { async run() { return { report: { status: "ok" } }; } };
  assert.equal(await callNurt(silent, '{"program":"count"}'), '{"status":"ok"}');
});

test("does not run the console when the tree is not JSON text", async () => {
  const nurt = new MockNurt();
  assert.match(await callNurt(nurt, '{"ast":"{"}'), /^error: /);
  assert.deepEqual(nurt.calls, []);
});

test("stops after the turn limit", async () => {
  let requests = 0;
  const call = { type: "function_call", name: "nurt", call_id: "c", arguments: '{"program":"count"}' };
  const client = { responses: { create: async () => (requests += 1, { id: `r${requests}`, output: [call], output_text: "" }) } };
  await assert.rejects(runAgent(client, new MockNurt(), { type: "function" }, "loop"), /no answer after 20 turns/);
  assert.equal(requests, 20);
});

test("reports malformed arguments to the model", async () => {
  assert.match(await callNurt(new MockNurt(), "{"), /^error: /);
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
