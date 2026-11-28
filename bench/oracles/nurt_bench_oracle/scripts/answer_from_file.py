# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bench/oracles/nurt_bench_oracle/scripts/answer_from_file.py
# Responsibility:   Script answer_from_file.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Checks that the final answer contains a value recorded in a file.

Used when the expected answer is known only at run time, for example the
PID of a process started by ``setup``.

Parameters (``oracle.params``):

- ``path``: file relative to the task root;
- ``state``: ``before`` (default) or ``after``;
- ``line``: optional zero-based line of the file; the whole file otherwise.

The value is the stripped text of the file or line.
"""

from __future__ import annotations

import re
from typing import Any, Mapping

from ..snapshot import Index
from ..textform import decode, split_lines


def check(
    before: Index, after: Index, task: Mapping[str, Any], observation: Mapping[str, Any]
) -> dict[str, Any]:
    """Evaluates the final answer against the recorded value."""
    params = task["oracle"]["params"]
    state = after if params.get("state") == "after" else before
    entry = state.get(params["path"])
    if entry is None or entry.kind != "file":
        return {"completed": False, "failures": [f"{params['path']}: value file missing"]}
    text = decode(entry.content or b"")
    if "line" in params:
        lines = split_lines(text)
        if params["line"] >= len(lines):
            return {"completed": False, "failures": [f"{params['path']}: line missing"]}
        text = lines[params["line"]]
    value = text.strip()
    answer = observation.get("answer") or ""
    if not value or re.search(rf"(?<![\w.]){re.escape(value)}(?![\w.])", answer) is None:
        return {"completed": False, "failures": [f"answer does not contain {value!r}"]}
    return {"completed": True, "failures": []}

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
