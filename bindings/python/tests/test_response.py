# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bindings/python/tests/test_response.py
# Responsibility:   Script test_response.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Access to the members of a response."""

from __future__ import annotations

import unittest

import support  # noqa: F401  (puts the package on the import path)

from nurt._response import Response


class ReportsTest(unittest.TestCase):
    def test_a_script_response_lists_the_report_of_every_pipeline(self) -> None:
        response = Response(
            {
                "report": {"status": "ok", "kind": "remove"},
                "reports": [
                    {"status": "ok", "kind": "find"},
                    {"status": "ok", "kind": "remove"},
                    "not a report",
                ],
            }
        )
        self.assertEqual([report["kind"] for report in response.reports], ["find", "remove"])

    def test_a_single_operation_has_no_reports(self) -> None:
        self.assertEqual(Response({"report": {"status": "ok", "kind": "find"}}).reports, [])
        self.assertEqual(Response({"report": {}, "reports": "x"}).reports, [])


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
