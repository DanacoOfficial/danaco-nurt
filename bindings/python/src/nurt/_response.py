# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Results of calls."""

from __future__ import annotations

import json
from typing import Any, Dict, List, Mapping, Optional, cast

from .types import Error, Report


class Response:
    """Response to a request (``nurt-response/1``).

    A response with a language error is still a response: ``ok`` is ``False``
    and ``error`` holds the ``error`` object of the report.
    """

    __slots__ = ("raw",)

    def __init__(self, raw: Mapping[str, Any]) -> None:
        self.raw: Dict[str, Any] = dict(raw)
        """The complete response as decoded JSON."""

    @property
    def report(self) -> Report:
        """Effect report (``nurt-report/1``)."""
        return cast(Report, self.raw.get("report", {}))

    @property
    def text(self) -> str:
        """Rendered text of the report; the report as JSON for the ``json`` format."""
        rendered = self.raw.get("rendered")
        if isinstance(rendered, Mapping) and isinstance(rendered.get("text"), str):
            return str(rendered["text"])
        return json.dumps(self.report, ensure_ascii=False, indent=2)

    @property
    def ok(self) -> bool:
        """``True`` when the report has no ``error``."""
        return "error" not in self.report

    @property
    def error(self) -> Optional[Error]:
        """The ``error`` object of the report, or ``None``."""
        return self.report.get("error")

    @property
    def next_cursor(self) -> Optional[str]:
        """Cursor of the next page, or ``None`` on the last page."""
        return self.report.get("next_cursor")

    def __repr__(self) -> str:
        status = self.report.get("status")
        return f"Response(status={status!r}, ok={self.ok!r})"


class CheckResult:
    """Result of :meth:`nurt.Nurt.check`: binding and effect classification."""

    __slots__ = ("raw", "diagnostics")

    def __init__(self, raw: Mapping[str, Any], diagnostics: List[Dict[str, Any]]) -> None:
        self.raw: Dict[str, Any] = dict(raw)
        """The complete result (``nurt-check/1``) as decoded JSON."""
        self.diagnostics = diagnostics
        """Diagnostics of the binding stage."""

    @property
    def bound(self) -> Optional[Dict[str, Any]]:
        """Bound tree (``nurt-bound/1``), or ``None`` when binding failed."""
        bound = self.raw.get("bound")
        return dict(bound) if isinstance(bound, Mapping) else None

    def __repr__(self) -> str:
        return f"CheckResult(bound={self.bound is not None!r}, diagnostics={len(self.diagnostics)})"


class ParseResult:
    """Result of :meth:`nurt.Nurt.parse`."""

    __slots__ = ("ast", "diagnostics")

    def __init__(self, ast: Optional[Dict[str, Any]], diagnostics: List[Dict[str, Any]]) -> None:
        self.ast = ast
        """Syntax tree (``nurt-ast/1``), or ``None`` when the program does not parse."""
        self.diagnostics = diagnostics
        """Diagnostics with positions and messages."""

    @property
    def ok(self) -> bool:
        """``True`` when the program parsed into a syntax tree."""
        return self.ast is not None

    def __repr__(self) -> str:
        return f"ParseResult(ok={self.ok!r}, diagnostics={len(self.diagnostics)})"


class CorrectResult:
    """Result of :meth:`nurt.Nurt.correct`."""

    __slots__ = ("raw",)

    def __init__(self, raw: Mapping[str, Any]) -> None:
        self.raw: Dict[str, Any] = dict(raw)
        """The complete result as decoded JSON."""

    @property
    def program(self) -> str:
        """The corrected program text."""
        return str(self.raw.get("program", ""))

    @property
    def corrections(self) -> List[Dict[str, Any]]:
        """Corrections applied to the program text."""
        corrections = self.raw.get("corrections", [])
        return list(corrections) if isinstance(corrections, list) else []

    def __repr__(self) -> str:
        return f"CorrectResult(corrections={len(self.corrections)})"
