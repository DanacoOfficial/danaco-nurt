<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Anthropic SDK (TypeScript)

Agent loop with the Anthropic Messages API and the `nurt` tool in the `anthropic` format.

## Requirements

- Node.js 24 or later, the `nurt` executable in `PATH`.
- `ANTHROPIC_API_KEY` in the environment; `ANTHROPIC_MODEL` selects the model (default `claude-sonnet-4-5`).

## Usage

```sh
npm install
node agent.ts "count the Python files under /home/user/project"
```

The arguments of a `tool_use` block are split into the input (`program`, `ast`, `cursor`, `accept`) and the options of `Nurt.run`.

## Test

```sh
npm test
```
