# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bindings/python/src/nurt/_response.py
# Responsibility:   Script _response.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Results of calls."""

from __future__ import annotations

import json
from typing import Any, Dict, List, Mapping, Optional, cast

from ._errors import NurtError, NurtLanguageError
from .types import Error, Report

UNKNOWN_VALUE_CODE = "E_UNKNOWN_VALUE"
"""Language error code of an argument value that the engine does not know."""


def _diagnostics(raw: Mapping[str, Any]) -> List[Dict[str, Any]]:
    value = raw.get("diagnostics")
    return [item for item in value if isinstance(item, dict)] if isinstance(value, list) else []


def _error(raw: Mapping[str, Any]) -> Optional[Dict[str, Any]]:
    """Language error of a result: the member ``error`` or the ``error`` of the report of a failure response."""
    error = raw.get("error")
    if not isinstance(error, Mapping):
        report = raw.get("report")
        error = report.get("error") if isinstance(report, Mapping) else None
    return dict(error) if isinstance(error, Mapping) else None


def failure_response(kind: str, error: Mapping[str, Any]) -> Dict[str, Any]:
    """Response ``nurt-response/1`` of a language error of the method ``kind``, as ``nurt serve`` answers."""
    report = {"nurt": "1.0", "status": "failed", "kind": kind, "error": dict(error)}
    return {"nurt": "1.0", "report": report}


def query_result(result: Dict[str, Any]) -> Dict[str, Any]:
    """Return the result of a query, or raise its language error.

    The server reports a language error of every method as a response whose
    report has the status ``failed``. A query has no response to return it
    in, so the error is raised: as ``NURT_E_ARG`` when it names an unknown
    argument value, as :class:`NurtLanguageError` otherwise.
    """
    report = result.get("report")
    if isinstance(report, dict) and report.get("status") == "failed" and isinstance(report.get("error"), dict):
        if report["error"].get("code") == UNKNOWN_VALUE_CODE:
            raise NurtError("NURT_E_ARG", str(report["error"].get("message", "")))
        raise NurtLanguageError(Response(result))
    return result


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
    def reports(self) -> List[Report]:
        """Reports of the pipelines of a script that ran several operations, in program order.

        Empty for a program with a single operation, whose ``report`` holds its
        records. The reports are not paged and have no ``next_cursor``.
        """
        reports = self.raw.get("reports")
        if not isinstance(reports, list):
            return []
        return [cast(Report, item) for item in reports if isinstance(item, dict)]

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

    __slots__ = ("raw",)

    def __init__(self, raw: Mapping[str, Any]) -> None:
        self.raw: Dict[str, Any] = dict(raw)
        """The complete result (``nurt-check/1``) as decoded JSON."""

    @property
    def bound(self) -> Optional[Dict[str, Any]]:
        """Bound tree (``nurt-bound/1``) when the result carries one, otherwise ``None``."""
        bound = self.raw.get("bound")
        return dict(bound) if isinstance(bound, Mapping) else None

    @property
    def diagnostics(self) -> List[Dict[str, Any]]:
        """Diagnostics of the binding stage."""
        return _diagnostics(self.raw)

    @property
    def error(self) -> Optional[Dict[str, Any]]:
        """Language error that stopped the binding, or ``None``."""
        return _error(self.raw)

    @property
    def ok(self) -> bool:
        """``True`` when the check produced a result without a language error.

        The result is the effect summary of ``nurt-check/1`` (member ``effect``);
        a result with a bound tree counts as well.
        """
        return self.error is None and ("effect" in self.raw or self.bound is not None)

    def __repr__(self) -> str:
        return f"CheckResult(ok={self.ok!r}, diagnostics={len(self.diagnostics)})"


class ParseResult:
    """Result of :meth:`nurt.Nurt.parse`."""

    __slots__ = ("raw",)

    def __init__(self, raw: Mapping[str, Any]) -> None:
        self.raw: Dict[str, Any] = dict(raw)
        """The complete result as decoded JSON."""

    @property
    def ast(self) -> Optional[Dict[str, Any]]:
        """Syntax tree (``nurt-ast/1``), or ``None`` when the program does not parse."""
        ast = self.raw.get("ast")
        return dict(ast) if isinstance(ast, Mapping) else None

    @property
    def diagnostics(self) -> List[Dict[str, Any]]:
        """Diagnostics with positions and messages."""
        return _diagnostics(self.raw)

    @property
    def error(self) -> Optional[Dict[str, Any]]:
        """Language error that stopped the parse, or ``None``."""
        return _error(self.raw)

    @property
    def ok(self) -> bool:
        """``True`` when the program parsed into a syntax tree."""
        return self.error is None and self.ast is not None

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

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
