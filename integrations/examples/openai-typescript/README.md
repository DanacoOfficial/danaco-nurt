<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# OpenAI SDK (TypeScript)

Agent loop with the OpenAI Responses API and the `nurt` tool in the `openai-responses-strict` format.

## Requirements

- Node.js 24 or later, the `nurt` executable in `PATH`.
- `OPENAI_API_KEY` in the environment; `OPENAI_MODEL` selects the model (default `gpt-5`).

## Usage

```sh
npm install
node agent.ts "count the Python files under /home/user/project"
```

Each turn sends the outputs of the function calls with `previous_response_id`. In the strict format `null` means an absent argument and `ast` is JSON text; `callNurt` removes `null` values and parses the tree.

## Test

```sh
npm test
```
