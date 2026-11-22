<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Vercel AI SDK

`generateText` with the `nurt` tool built from the neutral `json-schema` definition.

## Requirements

- Node.js 24 or later, the `nurt` executable in `PATH`.
- `ANTHROPIC_API_KEY` in the environment; `ANTHROPIC_MODEL` selects the model (default `claude-sonnet-4-5`). Any provider package of the AI SDK can replace `@ai-sdk/anthropic`.

## Usage

```sh
npm install
node agent.ts "count the Python files under /home/user/project"
```

`jsonSchema` wraps the input schema of the definition; `stopWhen: stepCountIs(20)` limits the number of steps of the loop.

## Test

```sh
npm test
```

The test uses `MockLanguageModelV2` from `ai/test`.
