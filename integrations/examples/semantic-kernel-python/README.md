<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           integrations/examples/semantic-kernel-python/README.md -->
<!-- Responsibility:   Documents Semantic Kernel (Python). -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Semantic Kernel (Python)

Semantic Kernel chat completion agent that uses the `nurt` tool of the MCP server `nurt mcp`.

## Requirements

- Python 3.10 or later, the `nurt` executable in `PATH` or in `NURT_EXECUTABLE`.
- `OPENAI_API_KEY` in the environment; `OPENAI_MODEL` selects the model (default `gpt-5`).

## Usage

```sh
pip install -r requirements.txt
python agent.py "count the Python files under /home/user/project"
```

`MCPStdioPlugin` starts `nurt mcp --stdio` and registers the tools of the server as kernel functions.

## Test

```sh
pip install "semantic-kernel[mcp]"
python -m unittest discover -s ../tests
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
