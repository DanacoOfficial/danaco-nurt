# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Oracles of the nurt-bench task set.

``check(before, after, task, observation)`` compares the final state of a
run with the expectation of a task and returns a verdict in the format
``nurt-bench-verdict/1``.
"""

from .oracle import VERDICT_FORMAT, check

__all__ = ["VERDICT_FORMAT", "check"]
