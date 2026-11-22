// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Agent loop with the Anthropic Messages API and the nurt tool.
// Usage: node agent.ts "count the Python files under /home/user/project"

import { execFileSync } from "node:child_process";

const MODEL = process.env["ANTHROPIC_MODEL"] ?? "claude-sonnet-4-5";
const MAX_TOKENS = 4096;
const MAX_TURNS = 20;
const INPUTS = new Set(["program", "ast", "cursor", "accept"]);

type Arguments = Record<string, unknown>;
interface NurtResponse { readonly text?: string | undefined; readonly ok: boolean; readonly report: unknown }
interface Console { run(input: Arguments, options: Arguments): Promise<NurtResponse> }
interface Block { type: string; id?: string; name?: string; input?: Arguments; text?: string }
interface Messages { create(request: Record<string, unknown>): Promise<{ content: Block[] }> }

export function toolDefinition(format: string): Record<string, unknown> {
  return JSON.parse(execFileSync("nurt", ["schema", "tool", "--format", format], { encoding: "utf8" }));
}

export async function callNurt(nurt: Console, args: Arguments): Promise<[string, boolean]> {
  const input: Arguments = {};
  const options: Arguments = {};
  for (const [name, value] of Object.entries(args)) {
    if (value !== null && value !== undefined) (INPUTS.has(name) ? input : options)[name] = value;
  }
  const response = await nurt.run(input, options);
  return [response.text ?? JSON.stringify(response.report), !response.ok];
}

export async function runAgent(client: { messages: Messages }, nurt: Console, tool: unknown, task: string): Promise<string> {
  const messages: Arguments[] = [{ role: "user", content: task }];
  for (let turn = 0; turn < MAX_TURNS; turn += 1) {
    const response = await client.messages.create({ model: MODEL, max_tokens: MAX_TOKENS, tools: [tool], messages });
    messages.push({ role: "assistant", content: response.content });
    const results: Arguments[] = [];
    for (const block of response.content) {
      if (block.type === "tool_use" && block.name === "nurt") {
        const [content, isError] = await callNurt(nurt, block.input ?? {});
        results.push({ type: "tool_result", tool_use_id: block.id, content, is_error: isError });
      }
    }
    if (results.length === 0) {
      return response.content.filter((block) => block.type === "text").map((block) => block.text).join("");
    }
    messages.push({ role: "user", content: results });
  }
  throw new Error(`no answer after ${MAX_TURNS} turns`);
}

async function main(): Promise<void> {
  const { default: Anthropic } = await import("@anthropic-ai/sdk");
  const { Nurt } = await import("nurt");
  const nurt = new Nurt();
  try {
    const answer = await runAgent(new Anthropic() as never, nurt as never, toolDefinition("anthropic"), process.argv.slice(2).join(" "));
    console.log(answer);
  } finally {
    await nurt.close();
  }
}

if (import.meta.main) await main();
