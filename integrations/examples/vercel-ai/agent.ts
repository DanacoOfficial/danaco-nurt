// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           integrations/examples/vercel-ai/agent.ts
// Responsibility:   Source file integrations/examples/vercel-ai/agent.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// Vercel AI SDK agent with the nurt tool.
// Usage: node agent.ts "count the Python files under /home/user/project"

import { execFileSync } from "node:child_process";

import { generateText, jsonSchema, stepCountIs, tool, type LanguageModel } from "ai";

const MAX_STEPS = 20;
const INPUTS = new Set(["program", "ast", "cursor", "accept"]);

type Arguments = Record<string, unknown>;
interface NurtResponse { readonly text?: string | undefined; readonly report: unknown }
interface Console { run(input: Arguments, options: Arguments): Promise<NurtResponse> }
interface Definition { name: string; description: string; input_schema: Record<string, unknown> }

export function toolDefinition(): Definition {
  return JSON.parse(execFileSync("nurt", ["schema", "tool", "--format", "json-schema"], { encoding: "utf8" }));
}

export function nurtTool(nurt: Console, definition: Definition) {
  return tool({
    description: definition.description,
    inputSchema: jsonSchema<Arguments>(definition.input_schema as never),
    execute: async (args: Arguments) => {
      const input: Arguments = {};
      const options: Arguments = {};
      for (const [name, value] of Object.entries(args)) {
        if (value !== null && value !== undefined) (INPUTS.has(name) ? input : options)[name] = value;
      }
      const response = await nurt.run(input, options);
      return response.text ?? JSON.stringify(response.report);
    },
  });
}

export async function runAgent(model: LanguageModel, nurt: Console, definition: Definition, task: string): Promise<string> {
  const result = await generateText({
    model,
    tools: { [definition.name]: nurtTool(nurt, definition) },
    stopWhen: stepCountIs(MAX_STEPS),
    prompt: task,
  });
  return result.text;
}

async function main(): Promise<void> {
  const { anthropic } = await import("@ai-sdk/anthropic");
  const { Nurt } = await import("nurt");
  const nurt = new Nurt();
  try {
    const model = anthropic(process.env["ANTHROPIC_MODEL"] ?? "claude-sonnet-4-5");
    console.log(await runAgent(model, nurt as never, toolDefinition(), process.argv.slice(2).join(" ")));
  } finally {
    await nurt.close();
  }
}

if (import.meta.main) await main();

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
