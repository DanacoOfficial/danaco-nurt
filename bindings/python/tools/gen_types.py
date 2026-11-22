# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Write ``src/nurt/types.py`` from the JSON schemas in ``spec/schemas``.

Usage::

    python3 tools/gen_types.py [--spec DIR] [--output FILE] [--check]

``--check`` compares the output with the existing file and exits with code 1
when they differ. Leading comment lines of the existing file (the file
header) are not compared and are kept when the file is written.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import textwrap
from pathlib import Path
from typing import Any, Dict, List, Mapping, Optional, Sequence, Set, Tuple

PACKAGE_DIR = Path(__file__).resolve().parent.parent
DEFAULT_SPEC = PACKAGE_DIR.parent.parent / "spec" / "schemas"
DEFAULT_OUTPUT = PACKAGE_DIR / "src" / "nurt" / "types.py"

SOURCES: Tuple[Tuple[str, str], ...] = (
    ("nurt-report-1.schema.json", "Report"),
    ("nurt-request-1.schema.json", "Request"),
)
"""Schema files and the Python name of their root object."""

NAMES: Dict[Tuple[str, str], str] = {
    ("nurt-report-1.schema.json", "status"): "ReportStatus",
    ("nurt-request-1.schema.json", "options"): "RequestOptions",
    ("nurt-request-1.schema.json", "response"): "ResponseData",
}
"""Python names of definitions whose plain name would be ambiguous."""

TYPES: Dict[Tuple[str, str], str] = {
    ("nurt-request-1.schema.json", "/$defs/response/properties/report"): "Report",
}
"""Locations that refer to another schema in prose only, with their Python type."""

LINE_LIMIT = 99
INDENT = "    "

TYPING_NAMES = ("Any", "Dict", "List", "Literal", "Optional", "Tuple", "TypedDict", "Union")
"""Names imported from :mod:`typing` when the module uses them."""

MODULE_DOC = '''"""Types of the JSON formats ``nurt-request/1``, ``nurt-response/1`` and ``nurt-report/1``.

The module is produced from ``spec/schemas`` with ``tools/gen_types.py``;
edit the schemas and run the tool instead of editing the module.
"""'''


def camel(name: str) -> str:
    """Convert a schema name to a class name, for example ``error_code`` to ``ErrorCode``."""
    return "".join(part[:1].upper() + part[1:] for part in name.replace("-", "_").split("_") if part)


class Generator:
    """Emits Python definitions for the schemas in dependency order."""

    def __init__(self) -> None:
        self.blocks: List[str] = []
        self.emitted: Dict[Tuple[str, str], str] = {}
        self.in_progress: Set[Tuple[str, str]] = set()
        self.request_options: Tuple[str, ...] = ()

    def add_schema(self, file_name: str, schema: Mapping[str, Any], root_name: str) -> None:
        """Emit the root object of ``schema`` and every definition."""
        definitions = schema.get("$defs", {})
        title = str(schema.get("title", file_name))
        self.emit_object(file_name, title, "", schema, root_name, definitions)
        for name in definitions:
            self.definition(file_name, title, name, definitions)
        if file_name == "nurt-request-1.schema.json":
            self.request_options = tuple(definitions["options"]["properties"])

    def definition(
        self, file_name: str, title: str, name: str, definitions: Mapping[str, Any]
    ) -> str:
        key = (file_name, name)
        python_name = NAMES.get(key, camel(name))
        if key in self.emitted:
            return self.emitted[key]
        if key in self.in_progress:
            return f'"{python_name}"'
        self.in_progress.add(key)
        schema = definitions[name]
        pointer = f"/$defs/{name}"
        if "enum" in schema:
            self.blocks.append(self.alias(python_name, literal(schema["enum"]), schema))
        elif schema.get("type") == "object" and schema.get("properties"):
            self.emit_object(file_name, title, pointer, schema, python_name, definitions)
        else:
            expression = self.type_of(file_name, title, pointer, schema, python_name, definitions)
            self.blocks.append(self.alias(python_name, expression, schema))
        self.in_progress.discard(key)
        self.emitted[key] = python_name
        return python_name

    def alias(self, name: str, expression: str, schema: Mapping[str, Any]) -> str:
        lines = [f"{name} = {expression}"]
        if len(lines[0]) > LINE_LIMIT and expression.startswith("Literal["):
            members = expression[len("Literal[") : -1].split(", ")
            lines = [f"{name} = Literal["] + [f"{INDENT}{member}," for member in members] + ["]"]
        description = schema.get("description")
        if description:
            lines.append(docstring(str(description), ""))
        return "\n".join(lines)

    def type_of(
        self,
        file_name: str,
        title: str,
        pointer: str,
        schema: Mapping[str, Any],
        context: str,
        definitions: Mapping[str, Any],
    ) -> str:
        override = TYPES.get((file_name, pointer))
        if override is not None:
            return override
        reference = schema.get("$ref")
        if isinstance(reference, str) and reference.startswith("#/$defs/"):
            return self.definition(file_name, title, reference[len("#/$defs/") :], definitions)
        if "enum" in schema:
            return literal(schema["enum"])
        kind = schema.get("type")
        if isinstance(kind, list):
            members = [
                self.type_of(file_name, title, pointer, {**schema, "type": k}, context, definitions)
                for k in kind
            ]
            optional = "None" in members
            members = [m for m in members if m != "None"]
            inner = members[0] if len(members) == 1 else f"Union[{', '.join(members)}]"
            return f"Optional[{inner}]" if optional else inner
        if kind == "string":
            return "str"
        if kind == "integer":
            return "int"
        if kind == "number":
            return "float"
        if kind == "boolean":
            return "bool"
        if kind == "null":
            return "None"
        if kind == "array":
            items = schema.get("items")
            if not isinstance(items, dict):
                return "List[Any]"
            item = self.type_of(file_name, title, f"{pointer}/items", items, f"{context}Item", definitions)
            return f"List[{item}]"
        if kind == "object":
            if schema.get("properties"):
                self.emit_object(file_name, title, pointer, schema, context, definitions)
                return context
            extra = schema.get("additionalProperties")
            if isinstance(extra, dict):
                value = self.type_of(
                    file_name, title, f"{pointer}/additionalProperties", extra, f"{context}Value", definitions
                )
                return f"Dict[str, {value}]"
            return "Dict[str, Any]"
        return "Any"

    def emit_object(
        self,
        file_name: str,
        title: str,
        pointer: str,
        schema: Mapping[str, Any],
        name: str,
        definitions: Mapping[str, Any],
    ) -> None:
        properties: Mapping[str, Any] = schema.get("properties", {})
        required = [p for p in schema.get("required", []) if p in properties]
        fields: Dict[str, str] = {}
        for prop, prop_schema in properties.items():
            fields[prop] = self.type_of(
                file_name, title, f"{pointer}/properties/{prop}", prop_schema, name + camel(prop), definitions
            )
        bases: List[str] = []
        mandatory = {k: v for k, v in fields.items() if k in required}
        optional = {k: v for k, v in fields.items() if k not in required}
        lines: List[str] = []
        if mandatory:
            lines.extend(typed_dict(f"_{name}Required", mandatory, total=True))
            bases.append(f"_{name}Required")
        if optional:
            lines.extend(typed_dict(f"_{name}Optional", optional, total=False))
            bases.append(f"_{name}Optional")
        description = schema.get("description") or f"Object ``{pointer or '/'}`` of ``{title}``."
        lines.append("")
        lines.append("")
        lines.append(f"class {name}({', '.join(bases)}):")
        lines.append(docstring(str(description), INDENT))
        self.blocks.append("\n".join(lines))


def literal(values: Sequence[Any]) -> str:
    return "Literal[" + ", ".join(json.dumps(v) for v in values) + "]"


def typed_dict(name: str, fields: Mapping[str, str], total: bool) -> List[str]:
    lines = [f"{name} = TypedDict(", f'{INDENT}"{name}",', f"{INDENT}{{"]
    for key, value in fields.items():
        lines.append(f'{INDENT * 2}"{key}": {value},')
    lines.append(f"{INDENT}}},")
    if not total:
        lines.append(f"{INDENT}total=False,")
    lines.append(")")
    return lines


def docstring(text: str, indent: str) -> str:
    text = " ".join(text.split()).replace("\\", "\\\\").replace('"""', '\\"\\"\\"')
    width = LINE_LIMIT - len(indent) - 6
    wrapped = textwrap.wrap(text, width=width) or [""]
    if len(wrapped) == 1:
        return f'{indent}"""{wrapped[0]}"""'
    body = "\n".join(f"{indent}{line}" for line in wrapped[1:])
    return f'{indent}"""{wrapped[0]}\n{body}\n{indent}"""'


def render(spec_dir: Path) -> str:
    """Return the text of ``types.py`` for the schemas in ``spec_dir``."""
    generator = Generator()
    for file_name, root_name in SOURCES:
        schema = json.loads((spec_dir / file_name).read_text(encoding="utf-8"))
        generator.add_schema(file_name, schema, root_name)
    body = "".join("\n\n\n" + block.lstrip("\n") for block in generator.blocks)
    names = "\n".join(f'{INDENT}"{name}",' for name in generator.request_options)
    body += (
        f"\n\n\nREQUEST_OPTIONS: Tuple[str, ...] = (\n{names}\n)\n"
        '"""Names of the options of ``nurt-request/1``."""\n'
    )
    used = [name for name in TYPING_NAMES if re.search(rf"\b{name}\b", body)]
    header = f"{MODULE_DOC}\n\nfrom __future__ import annotations\n\nfrom typing import {', '.join(used)}"
    return header + body


def strip_header(text: str) -> Tuple[str, str]:
    """Split ``text`` into its leading comment lines and the rest."""
    lines = text.splitlines(keepends=True)
    count = 0
    while count < len(lines) and lines[count].startswith("#"):
        count += 1
    return "".join(lines[:count]), "".join(lines[count:])


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--spec", type=Path, default=DEFAULT_SPEC, help="directory of the JSON schemas")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT, help="module to write")
    parser.add_argument("--check", action="store_true", help="fail when the module is out of date")
    arguments = parser.parse_args(argv)
    text = render(arguments.spec)
    header, current = "", ""
    if arguments.output.exists():
        header, current = strip_header(arguments.output.read_text(encoding="utf-8"))
    if arguments.check:
        if current != text:
            sys.stderr.write(f"{arguments.output} is out of date; run tools/gen_types.py\n")
            return 1
        return 0
    arguments.output.write_text(header + text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
