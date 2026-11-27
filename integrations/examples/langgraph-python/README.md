<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           integrations/examples/langgraph-python/README.md -->
<!-- Responsibility:   Documents LangChain and LangGraph (Python). -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# LangChain and LangGraph (Python)

LangChain agent, run by LangGraph, with the `nurt` tool built from the neutral `json-schema` definition.

## Requirements

- Python 3.10 or later, the `nurt` executable in `PATH`.
- The API key of the model provider in the environment (`ANTHROPIC_API_KEY` for the default model); `LANGCHAIN_MODEL` selects the model in the `provider:model` form (default `anthropic:claude-sonnet-4-5`).

## Usage

```sh
pip install -r requirements.txt
python agent.py "count the Python files under /home/user/project"
```

`StructuredTool` receives the input schema of the definition as `args_schema`, so the model sees the same arguments as in every other integration.

## Test

```sh
pip install langchain-core
python -m unittest discover -s ../tests
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
