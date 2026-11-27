# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bench/oracles/tests/test_schema.py
# Responsibility:   Script test_schema.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Tests of the validator of the task schema."""

from __future__ import annotations

import unittest

from nurt_bench_oracle.schema import SchemaError, Validator, json_equal, type_matches
from nurt_bench_oracle.snapshot import SnapshotError, check_relative


def errors_of(value: object, schema: dict[str, object]) -> list[str]:
    """Returns the errors of a value against a schema without references."""
    found: list[str] = []
    Validator({}).validate(value, schema, "$", found)
    return found


class EqualityTests(unittest.TestCase):
    def test_a_boolean_is_not_a_number(self) -> None:
        self.assertFalse(json_equal(True, 1))
        self.assertFalse(json_equal(0, False))
        self.assertTrue(json_equal(True, True))
        self.assertTrue(json_equal(1, 1.0))

    def test_booleans_inside_containers_are_not_numbers(self) -> None:
        self.assertFalse(json_equal([True], [1]))
        self.assertFalse(json_equal({"a": False}, {"a": 0}))
        self.assertTrue(json_equal({"a": [1, "x"]}, {"a": [1, "x"]}))
        self.assertFalse(json_equal({"a": 1}, {"b": 1}))


class KeywordTests(unittest.TestCase):
    def test_enum_does_not_accept_a_boolean_for_a_number(self) -> None:
        self.assertNotEqual(errors_of(True, {"enum": [1, 2]}), [])
        self.assertEqual(errors_of(1, {"enum": [1, 2]}), [])

    def test_const_does_not_accept_a_number_for_a_boolean(self) -> None:
        self.assertNotEqual(errors_of(1, {"const": True}), [])
        self.assertEqual(errors_of(True, {"const": True}), [])

    def test_an_unknown_type_is_a_schema_error_and_not_a_key_error(self) -> None:
        with self.assertRaisesRegex(SchemaError, "unknown type 'float'"):
            type_matches(1.5, "float")
        with self.assertRaises(SchemaError):
            errors_of("text", {"type": ["integer", "decimal"]})


class SnapshotPathTests(unittest.TestCase):
    def test_a_colon_inside_a_name_is_an_ordinary_character(self) -> None:
        self.assertEqual(check_relative("logs/a:b.txt"), "logs/a:b.txt")

    def test_a_drive_prefix_is_not_relative(self) -> None:
        for path in ("C:/x", "z:y", "C:"):
            with self.subTest(path=path), self.assertRaises(SnapshotError):
                check_relative(path)


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
