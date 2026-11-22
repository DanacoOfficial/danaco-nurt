# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Additional oracle scripts.

Each module provides ``check(before, after, task, observation)``, where
``before`` and ``after`` are indexes of the task root, and returns a
dictionary with ``completed`` (bool) and ``failures`` (list of text).
"""
