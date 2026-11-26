<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Tool definition

## Syntax

```
nurt schema tool [--format <format>] [--language en|pl] [--groups core|all|<group>,<group>...]
```

`nurt schema tool` without options prints the published schema `nurt-tool/1` (the schema of the `json-schema` format below), like every other name of `nurt schema`. With at least one option it prints the definition of the tool; an option that is not given takes its default. The description texts and the catalog version come from the installed engine.

## Parameters

| Parameter | Type | Default | Meaning |
|---|---|---|---|
| `--format` | enum | `json-schema` | output format, see below |
| `--language` | `en`, `pl` | `en` | language of the description; also the default of the `language` argument |
| `--groups` | text | `core` | command groups described in addition to the core group |

The value follows the option as a separate argument or after `=`. An unknown format, language or group is the error `E_UNKNOWN_VALUE` with the accepted values; an unknown option or a missing value is a usage error (exit code 2).

The Rust crate `nurt-tooldef` produces the same output: `render(ToolFormat, &RenderOptions)`. The output is deterministic for the same inputs.

## Result

One JSON object: the element of the tool list of the platform.

| Format | Platform | Object |
|---|---|---|
| `anthropic` | Anthropic Messages API | `{name, description, input_schema}` |
| `openai` | OpenAI Chat Completions | `{type: "function", function: {name, description, parameters, strict: false}}` |
| `openai-strict` | OpenAI Chat Completions, structured outputs | as `openai`, `strict: true` |
| `openai-responses` | OpenAI Responses API | `{type: "function", name, description, parameters, strict: false}` |
| `openai-responses-strict` | OpenAI Responses API, structured outputs | as `openai-responses`, `strict: true` |
| `gemini` | Gemini API | element of `functionDeclarations`: `{name, description, parameters}` |
| `mcp` | Model Context Protocol | element of `tools/list`: `{name, description, inputSchema, _meta}` |
| `json-schema` | any | `{name, description, input_schema, catalog_version}`; the input schema is JSON Schema 2020-12 |

The tool name is `nurt` in every format. The arguments are the inputs and the call options of `nurt-request/1`:

| Argument | Type | Meaning |
|---|---|---|
| `program` | text | program text |
| `ast` | object | syntax tree `nurt-ast/1`, equivalent to `program` |
| `cursor` | text | cursor of a further page of an earlier result |
| `accept` | text | identifier of a candidate of an earlier `E_AMBIGUOUS` error |
| `preview`, `base`, `limit`, `fields`, `format`, `verbosity`, `diff`, `diff_context`, `max_chars`, `save_to`, `timeout`, `session`, `language`, `script` | as in `nurt-request/1` | call options |

Exactly one of `program`, `ast`, `cursor` and `accept` is given. The envelope fields `nurt` (format version) and `variant` (notation variant, detected by the console) are not arguments.

## Rules per platform

The definition is generated from one source. Constructs that a platform does not accept are rewritten so that every format accepts the same calls; a constraint moved into a description is still checked by the console against `nurt-request/1`.

| Rule | Formats |
|---|---|
| the exclusive choice of `program`, `ast`, `cursor` and `accept` (`oneOf` at the root of the request schema) is stated in the root `description` | all |
| `additionalProperties: false` is removed | `gemini` |
| `default` is written into the description of the argument | `openai-strict`, `openai-responses-strict`, `gemini` |
| `minLength` is written into the description | `openai-strict`, `openai-responses-strict` |
| every argument is required and has `null` in its type; `null` means an absent argument | `openai-strict`, `openai-responses-strict` |
| `ast` is passed as JSON text | `openai-strict`, `openai-responses-strict`, `gemini` |
| enumerations carry `type: string`; type names are `STRING`, `INTEGER`, `OBJECT`, ... with `format: enum` | `gemini` (both), strict formats (first only) |

A host that receives arguments in a strict format or in `gemini` removes `null` values and parses `ast` text before it builds the request. The language bindings and `nurt_tooldef::request_from_arguments` do this.

## Description and command groups

The description holds the core description of the language, then the description of every selected group. The core group is always present. Other groups are loaded on demand:

1. Start with `--groups core`.
2. When the model needs a group, for example after `help group: "git"` or after `E_UNKNOWN_COMMAND` with a command of an optional group, render the definition again with the group added (`--groups git`), or append the text of `describe` for the group to the context.
3. MCP servers send `notifications/tools/list_changed` after the set of groups changes; clients read the tool list again.

`catalog_version` (`json-schema`) and `_meta["pl.danaco-group/nurt-catalog"]` (`mcp`) carry the version of the command catalog the description was produced from. A host that caches definitions uses the format, the language, the groups and this version as the cache key.

## Examples

```sh
nurt schema tool --format anthropic
nurt schema tool --format openai-responses-strict --groups core,git
nurt schema tool --format mcp --language pl --groups all
```
