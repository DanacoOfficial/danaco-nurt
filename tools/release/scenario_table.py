#!/usr/bin/env python3
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           tools/release/scenario_table.py
# Responsibility:   Script scenario_table.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Render the results of tools/release/scenarios.sh as a table.

The input is a text file with one result per line, three fields separated by
a tab: the scenario identifier, the status (PASS, FAIL or SKIP) and a
one-line reason. Empty lines are ignored.

The table has one row per scenario, in the order of the input, followed by a
summary line. The exit code is 0 when no scenario failed, 1 when at least one
failed, and 2 when the input is malformed.

Usage:

  scenario_table.py RESULTS_FILE
"""

import sys
from pathlib import Path

STATUSES = ("PASS", "FAIL", "SKIP")
REASON_LIMIT = 160


class ResultError(Exception):
    """A line of the results file that is not a valid result."""


def parse_results(text):
    """Return the list of (identifier, status, reason) of a results file."""
    results = []
    seen = set()
    for number, line in enumerate(text.splitlines(), start=1):
        if not line.strip():
            continue
        fields = line.split("\t")
        if len(fields) != 3:
            raise ResultError(f"line {number}: expected 3 tab-separated fields, found {len(fields)}")
        identifier, status, reason = (field.strip() for field in fields)
        if not identifier:
            raise ResultError(f"line {number}: empty scenario identifier")
        if identifier in seen:
            raise ResultError(f"line {number}: scenario {identifier} appears twice")
        if status not in STATUSES:
            raise ResultError(f"line {number}: status {status!r} is not PASS, FAIL or SKIP")
        if not reason:
            raise ResultError(f"line {number}: scenario {identifier} has no reason")
        if len(reason) > REASON_LIMIT:
            reason = reason[: REASON_LIMIT - 3] + "..."
        seen.add(identifier)
        results.append((identifier, status, reason))
    if not results:
        raise ResultError("no results")
    return results


def summarize(results):
    """Return the number of scenarios per status."""
    counts = {status: 0 for status in STATUSES}
    for _, status, _ in results:
        counts[status] += 1
    return counts


def render(results):
    """Return the table and the summary line as text."""
    width = max(len(identifier) for identifier, _, _ in results)
    width = max(width, len("Scenario"))
    lines = [f"{'Scenario':<{width}}  Result  Reason", f"{'-' * width}  ------  ------"]
    for identifier, status, reason in results:
        lines.append(f"{identifier:<{width}}  {status:<6}  {reason}")
    counts = summarize(results)
    lines.append("")
    lines.append(
        f"{len(results)} scenarios: {counts['PASS']} PASS, {counts['FAIL']} FAIL, {counts['SKIP']} SKIP"
    )
    return "\n".join(lines) + "\n"


def exit_code(results):
    """Return 1 when a scenario failed, otherwise 0."""
    return 1 if summarize(results)["FAIL"] else 0


def main(argv):
    if len(argv) != 2:
        print(__doc__.split("Usage:")[1].strip(), file=sys.stderr)
        return 2
    try:
        results = parse_results(Path(argv[1]).read_text(encoding="utf-8"))
    except (OSError, ResultError) as error:
        print(f"scenario_table: {error}", file=sys.stderr)
        return 2
    sys.stdout.write(render(results))
    return exit_code(results)


if __name__ == "__main__":
    sys.exit(main(sys.argv))

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
