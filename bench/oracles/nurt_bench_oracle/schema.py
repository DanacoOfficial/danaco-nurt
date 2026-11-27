# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bench/oracles/nurt_bench_oracle/schema.py
# Responsibility:   Script schema.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Validation of JSON values against the task schema.

Implements the subset of JSON Schema 2020-12 used by
``bench/tasks/schema/nurt-bench-task-1.schema.json``: ``type``, ``const``,
``enum``, ``pattern``, ``minLength``, ``minimum``, ``maximum``,
``required``, ``properties``, ``additionalProperties``, ``minProperties``,
``items``, ``minItems``, ``allOf``, ``oneOf``, ``not``, ``if``/``then``/
``else`` and local ``$ref``.
"""

from __future__ import annotations

import re
from typing import Any, Mapping

TYPES: dict[str, tuple[type, ...]] = {
    "object": (dict,),
    "array": (list,),
    "string": (str,),
    "integer": (int,),
    "number": (int, float),
    "boolean": (bool,),
    "null": (type(None),),
}


class SchemaError(ValueError):
    """The schema itself is invalid, for example it names an unknown type."""


def json_equal(first: Any, second: Any) -> bool:
    """Returns whether two JSON values are equal.

    Unlike ``==`` in Python, a boolean never equals a number (``True`` is not
    ``1``), also inside arrays and objects.
    """
    if isinstance(first, bool) or isinstance(second, bool):
        return isinstance(first, bool) and isinstance(second, bool) and first == second
    if isinstance(first, list) and isinstance(second, list):
        return len(first) == len(second) and all(json_equal(a, b) for a, b in zip(first, second))
    if isinstance(first, dict) and isinstance(second, dict):
        return first.keys() == second.keys() and all(json_equal(first[key], second[key]) for key in first)
    return bool(first == second)


class Validator:
    """Validates values against one root schema."""

    def __init__(self, root: Mapping[str, Any]) -> None:
        self.root = root

    def errors(self, value: Any) -> list[str]:
        """Returns the validation errors of a value; empty when valid."""
        found: list[str] = []
        self.validate(value, self.root, "$", found)
        return found

    def resolve(self, ref: str) -> Mapping[str, Any]:
        """Resolves a local reference of the form ``#/$defs/name``."""
        node: Any = self.root
        for part in ref.removeprefix("#/").split("/"):
            node = node[part]
        return node

    def is_valid(self, value: Any, schema: Mapping[str, Any]) -> bool:
        """Returns whether a value satisfies a subschema."""
        found: list[str] = []
        self.validate(value, schema, "$", found)
        return not found

    def validate(self, value: Any, schema: Mapping[str, Any], where: str, found: list[str]) -> None:
        """Appends to ``found`` the errors of ``value`` at location ``where``."""
        if "$ref" in schema:
            self.validate(value, self.resolve(schema["$ref"]), where, found)
        if "type" in schema and not type_matches(value, schema["type"]):
            found.append(f"{where}: expected {schema['type']}")
            return
        if "const" in schema and not json_equal(value, schema["const"]):
            found.append(f"{where}: expected {schema['const']!r}")
        if "enum" in schema and not any(json_equal(value, option) for option in schema["enum"]):
            found.append(f"{where}: {value!r} not in {schema['enum']}")
        if isinstance(value, str):
            self.validate_string(value, schema, where, found)
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            if "minimum" in schema and value < schema["minimum"]:
                found.append(f"{where}: below {schema['minimum']}")
            if "maximum" in schema and value > schema["maximum"]:
                found.append(f"{where}: above {schema['maximum']}")
        if isinstance(value, dict):
            self.validate_object(value, schema, where, found)
        if isinstance(value, list):
            if len(value) < schema.get("minItems", 0):
                found.append(f"{where}: fewer than {schema['minItems']} items")
            if "items" in schema:
                for position, item in enumerate(value):
                    self.validate(item, schema["items"], f"{where}[{position}]", found)
        self.validate_combinators(value, schema, where, found)

    def validate_string(
        self, value: str, schema: Mapping[str, Any], where: str, found: list[str]
    ) -> None:
        """Validates the string keywords."""
        if len(value) < schema.get("minLength", 0):
            found.append(f"{where}: shorter than {schema['minLength']}")
        if "pattern" in schema and re.search(schema["pattern"], value) is None:
            found.append(f"{where}: {value!r} does not match {schema['pattern']}")

    def validate_object(
        self, value: dict[str, Any], schema: Mapping[str, Any], where: str, found: list[str]
    ) -> None:
        """Validates the object keywords."""
        for name in schema.get("required", []):
            if name not in value:
                found.append(f"{where}: missing {name}")
        if len(value) < schema.get("minProperties", 0):
            found.append(f"{where}: fewer than {schema['minProperties']} properties")
        properties = schema.get("properties", {})
        extra = schema.get("additionalProperties", True)
        for name, item in value.items():
            if name in properties:
                self.validate(item, properties[name], f"{where}.{name}", found)
            elif extra is False:
                found.append(f"{where}: unexpected property {name}")
            elif isinstance(extra, Mapping):
                self.validate(item, extra, f"{where}.{name}", found)

    def validate_combinators(
        self, value: Any, schema: Mapping[str, Any], where: str, found: list[str]
    ) -> None:
        """Validates ``allOf``, ``oneOf``, ``not`` and ``if``."""
        for sub in schema.get("allOf", []):
            self.validate(value, sub, where, found)
        if "oneOf" in schema:
            hits = sum(1 for sub in schema["oneOf"] if self.is_valid(value, sub))
            if hits != 1:
                found.append(f"{where}: matches {hits} alternatives of oneOf, expected 1")
        if "not" in schema and self.is_valid(value, schema["not"]):
            found.append(f"{where}: matches a forbidden schema")
        if "if" in schema:
            branch = "then" if self.is_valid(value, schema["if"]) else "else"
            if branch in schema:
                self.validate(value, schema[branch], where, found)


def type_matches(value: Any, expected: str | list[str]) -> bool:
    """Returns whether a value has one of the JSON types."""
    names = [expected] if isinstance(expected, str) else expected
    for name in names:
        if isinstance(value, bool) and name in ("integer", "number"):
            continue
        types = TYPES.get(name)
        if types is None:
            raise SchemaError(f"unknown type {name!r} in a schema")
        if isinstance(value, types):
            return True
    return False

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
