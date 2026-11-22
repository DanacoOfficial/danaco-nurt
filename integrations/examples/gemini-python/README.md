<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Google Gen AI SDK (Python)

Agent loop with the Gemini API and the `nurt` tool in the `gemini` format.

## Requirements

- Python 3.10 or later, the `nurt` executable in `PATH`.
- `GEMINI_API_KEY` in the environment; `GEMINI_MODEL` selects the model (default `gemini-2.5-flash`).

## Usage

```sh
pip install -r requirements.txt
python agent.py "count the Python files under /home/user/project"
```

Automatic function calling is disabled: the loop runs each function call with `Nurt.run` and returns the output or the error as the function response. The `gemini` format passes `ast` as JSON text, because the schema subset of the API has no object without declared properties.

## Test

```sh
python -m unittest discover -s ../tests
```
