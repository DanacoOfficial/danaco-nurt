<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# OpenAI SDK (Python)

Agent loop with the OpenAI Chat Completions API and the `nurt` tool in the `openai-strict` format.

## Requirements

- Python 3.10 or later, the `nurt` executable in `PATH`.
- `OPENAI_API_KEY` in the environment; `OPENAI_MODEL` selects the model (default `gpt-5`); `NURT_TOOL_FORMAT=openai` selects the format without structured outputs.

## Usage

```sh
pip install -r requirements.txt
python agent.py "count the Python files under /home/user/project"
```

In the strict format the model sends every argument and `null` for an absent one, and sends `ast` as JSON text. `Nurt.run` ignores `null` options and parses `ast` text, so the arguments are passed unchanged.

## Test

```sh
python -m unittest discover -s ../tests
```
