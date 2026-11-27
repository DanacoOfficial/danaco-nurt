// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           integrations/examples/openai-typescript/agent.ts
// Responsibility:   Source file integrations/examples/openai-typescript/agent.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// Agent loop with the OpenAI Responses API, the strict nurt tool and function calling.
// Usage: node agent.ts "count the Python files under /home/user/project"

import { execFileSync } from "node:child_process";

const MODEL = process.env["OPENAI_MODEL"] ?? "gpt-5";
const MAX_TURNS = 20;
const INPUTS = new Set(["program", "ast", "cursor", "accept"]);

type Arguments = Record<string, unknown>;
interface NurtResponse { readonly text?: string | undefined; readonly report: unknown }
interface Console { run(input: Arguments, options: Arguments): Promise<NurtResponse> }
interface Item { type: string; name?: string; call_id?: string; arguments?: string }
interface Responses { create(request: Record<string, unknown>): Promise<{ id: string; output: Item[]; output_text: string }> }

export function toolDefinition(format: string): Record<string, unknown> {
  return JSON.parse(execFileSync("nurt", ["schema", "tool", "--format", format], { encoding: "utf8" }));
}

// The strict format sends every argument, with null for an absent one, and
// passes `ast` as JSON text.
export async function callNurt(nurt: Console, text: string): Promise<string> {
  let args: Arguments;
  try {
    args = JSON.parse(text) as Arguments;
    if (typeof args["ast"] === "string") args["ast"] = JSON.parse(args["ast"]);
  } catch (error) {
    return `error: ${(error as Error).message}`;
  }
  const input: Arguments = {};
  const options: Arguments = {};
  for (const [name, value] of Object.entries(args)) {
    if (value !== null && value !== undefined) (INPUTS.has(name) ? input : options)[name] = value;
  }
  const response = await nurt.run(input, options);
  return response.text ?? JSON.stringify(response.report);
}

export async function runAgent(client: { responses: Responses }, nurt: Console, tool: unknown, task: string): Promise<string> {
  let input: Arguments[] = [{ role: "user", content: task }];
  let previous: string | undefined;
  for (let turn = 0; turn < MAX_TURNS; turn += 1) {
    const response = await client.responses.create({ model: MODEL, tools: [tool], input, previous_response_id: previous });
    const calls = response.output.filter((item) => item.type === "function_call");
    if (calls.length === 0) return response.output_text;
    input = [];
    for (const call of calls) {
      const output = call.name === "nurt" ? await callNurt(nurt, call.arguments ?? "{}") : `error: unknown tool ${call.name}`;
      input.push({ type: "function_call_output", call_id: call.call_id, output });
    }
    previous = response.id;
  }
  throw new Error(`no answer after ${MAX_TURNS} turns`);
}

async function main(): Promise<void> {
  const { default: OpenAI } = await import("openai");
  const { Nurt } = await import("nurt");
  const nurt = new Nurt();
  try {
    const tool = toolDefinition("openai-responses-strict");
    console.log(await runAgent(new OpenAI() as never, nurt as never, tool, process.argv.slice(2).join(" ")));
  } finally {
    await nurt.close();
  }
}

if (import.meta.main) await main();

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
