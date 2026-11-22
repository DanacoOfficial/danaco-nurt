<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Anthropic SDK (Python)

Agent loop with the Anthropic Messages API and the `nurt` tool in the `anthropic` format.

## Requirements

- Python 3.10 or later, the `nurt` executable in `PATH`.
- `ANTHROPIC_API_KEY` in the environment; `ANTHROPIC_MODEL` selects the model (default `claude-sonnet-4-5`).

## Usage

```sh
pip install -r requirements.txt
python agent.py "count the Python files under /home/user/project"
```

The definition comes from `nurt schema tool --format anthropic`. Each `tool_use` block is run with `Nurt.run`; a report with a language error is returned with `is_error: true`, so the model receives the correction of the error.

## Test

```sh
python -m unittest discover -s ../tests
```
