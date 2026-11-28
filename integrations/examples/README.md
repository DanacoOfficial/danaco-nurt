<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           integrations/examples/README.md -->
<!-- Responsibility:   Documents Integration examples. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Integration examples

Minimal agent loops and client configurations that give a model the `nurt` tool. Every example uses the same tool name and the same arguments; the definition is rendered by `nurt schema tool --format <format>`.

| Directory | Platform | Language | Tool definition | Console access |
|---|---|---|---|---|
| `anthropic-python` | Anthropic SDK | Python | `anthropic` | `nurt` package |
| `anthropic-typescript` | Anthropic SDK | TypeScript | `anthropic` | `nurt` package |
| `openai-python` | OpenAI SDK, Chat Completions | Python | `openai-strict` | `nurt` package |
| `openai-typescript` | OpenAI SDK, Responses API | TypeScript | `openai-responses-strict` | `nurt` package |
| `gemini-python` | Google Gen AI SDK | Python | `gemini` | `nurt` package |
| `vercel-ai` | Vercel AI SDK | TypeScript | `json-schema` | `nurt` package |
| `langgraph-python` | LangChain and LangGraph | Python | `json-schema` | `nurt` package |
| `semantic-kernel-python` | Semantic Kernel | Python | `mcp` | `nurt mcp` |
| `openhands` | OpenHands | configuration | `mcp` | `nurt mcp` |
| `swe-agent` | SWE-agent | tool bundle | command `nurt` | `nurt exec` |
| `mcp` | MCP clients | configuration | `mcp` | `nurt mcp` |

API keys are read from environment variables only. The tests replace the model and the console with mock objects and need neither network access nor keys:

```sh
python -m unittest discover -s integrations/examples/tests
(cd integrations/examples/anthropic-typescript && npm test)
```

The workflow `.github/workflows/integrations-examples.yml` (its copy in this directory is `ci/workflow.yml`) runs the tests of every example. The integrator guide is in `docs/integrators/`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
