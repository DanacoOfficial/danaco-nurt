---
title: Integration
order: 9
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Integration

A host program can call Danaco Nurt in four ways. The tool seen by a model is named `nurt` in every integration.

| Interface | Use |
|---|---|
| MCP server | `nurt mcp --stdio` or `nurt mcp --http <address>` provides the tool `nurt` |
| JSON-RPC | `nurt serve --stdio` or `nurt serve --socket <path>` speaks `nurt-rpc/1` |
| C library | `libnurt` and `nurt.h`; the bindings for other languages use it |
| Rust library | the crate `nurt-core` |

## The tool

The tool takes a program in text, or a syntax tree, and returns the effect report. The schema of the tool is `nurt-tool/1`; `nurt schema tool` prints it. The description for the model is produced by the console from the catalog, so it always agrees with the installed version.

```text
nurt schema tool
```

## Policies of the engine

The command `nurt check` and the library function `nurt_check` return the effects, paths, targets, external programs, network hosts and groups of a program without running it. An engine applies its own permission policy to this result before it calls the tool.

## Documentation for integrators

The guide for integrators, with tool definitions for the model providers and examples of policies, is the directory [docs/integrators](https://github.com/DanacoOfficial/nurt/tree/main/docs/integrators) of the repository.
