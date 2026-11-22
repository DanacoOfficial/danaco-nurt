<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Provider schemas

JSON Schema documents that describe the tool objects accepted by each
platform. The tests of `nurt-tooldef` validate every rendered definition
against the schema of its format, without network access.

The schemas are written for this project from the public documentation of
each platform; they contain no text copied from that documentation and are
distributed under the license of the repository. They cover the fields that
the generator writes and the constraints that the documentation states. A
platform may accept more than a schema allows; it must not reject a
definition that its schema accepts.

| File | Format | Source |
|---|---|---|
| `anthropic-tool.schema.json` | `anthropic` | Anthropic, Messages API reference, `tools` (custom tool), and the tool use guide: <https://docs.anthropic.com/en/docs/agents-and-tools/tool-use/implement-tool-use> |
| `openai-chat-tool.schema.json` | `openai`, `openai-strict` | OpenAI, Chat Completions API reference, `tools`; function calling guide: <https://platform.openai.com/docs/guides/function-calling> |
| `openai-responses-tool.schema.json` | `openai-responses`, `openai-responses-strict` | OpenAI, Responses API reference, function tool; function calling guide (default of `strict` in the Responses API) |
| `openai-strict-parameters.schema.json` | `parameters` of `openai-strict` and `openai-responses-strict` | OpenAI, structured outputs guide, supported schemas: <https://platform.openai.com/docs/guides/structured-outputs> |
| `gemini-function-declaration.schema.json` | `gemini` | Google, Gemini API reference, `FunctionDeclaration` and `Schema`: <https://ai.google.dev/api/caching#FunctionDeclaration> |
| `mcp-tool.schema.json` | `mcp` | Model Context Protocol specification, revision 2025-11-25, server tools: <https://modelcontextprotocol.io/specification/2025-11-25/server/tools> |
| `neutral-tool.schema.json` | `json-schema` | Neutral definition `nurt-tool/1` of this project |

Constraints recorded in the schemas:

| Format | Constraint |
|---|---|
| `anthropic` | tool name `^[a-zA-Z0-9_-]{1,64}$`; `input_schema` is an object schema without `oneOf`, `anyOf` or `allOf` at the top level |
| `openai` | function name `^[a-zA-Z0-9_-]{1,64}$`; `parameters` is an object schema and not a union |
| `openai-strict` | every object sets `additionalProperties: false` and lists every property in `required`; optional values are typed with `null`; keywords limited to the supported subset (no `default`, no `minLength`); the root is not `anyOf` |
| `openai-responses` | as `openai`; `strict` is given explicitly, because the Responses API otherwise converts the schema into strict mode when it can |
| `gemini` | function name `^[a-zA-Z0-9_:.-]{1,128}$`; `parameters` uses the fields of `Schema` only (no `additionalProperties`); type names are `STRING`, `INTEGER`, `OBJECT` and the other enumeration values; an `OBJECT` declares at least one property; `enum` applies to `STRING`; `default` is accepted and ignored |
| `mcp` | tool name 1 to 128 characters of `A-Z`, `a-z`, `0-9`, `_`, `-` and `.`; `inputSchema` is an object schema; `_meta` keys use an optional reverse-DNS prefix ending in `/` |
