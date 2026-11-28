# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bindings/python/src/nurt/_errors.py
# Responsibility:   Script _errors.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Exceptions raised by the binding."""

from __future__ import annotations

from typing import Any, Dict, Mapping, Optional

STATUS_NAMES: Dict[int, str] = {
    0: "NURT_OK",
    1: "NURT_E_TIMEOUT",
    2: "NURT_E_CANCELLED",
    3: "NURT_E_ARG",
    4: "NURT_E_ABI",
    5: "NURT_E_IO",
    6: "NURT_E_INTERNAL",
}
"""Names of the ``nurt_status`` codes of the C interface, by value."""

STATUS_CODES: Dict[str, int] = {name: code for code, name in STATUS_NAMES.items()}
"""Values of the ``nurt_status`` codes of the C interface, by name."""

UNKNOWN_STATUS = "NURT_E_UNKNOWN"
"""Status name used for a code that this version of the binding does not know."""


def status_name(code: int) -> str:
    """Return the name of a ``nurt_status`` code, or ``NURT_E_UNKNOWN``."""
    return STATUS_NAMES.get(code, UNKNOWN_STATUS)


class NurtError(Exception):
    """Failure of a call: the call itself did not complete.

    ``status`` is the name of the ``nurt_status`` code (for example
    ``"NURT_E_ARG"``), ``code`` its numeric value or ``None`` for an unknown
    name, and ``detail`` the structured detail reported by the implementation.
    Errors of the language are not call failures; see :class:`NurtLanguageError`.
    """

    def __init__(self, status: str, message: str, *, detail: Any = None) -> None:
        super().__init__(message)
        self.status = status
        self.code: Optional[int] = STATUS_CODES.get(status)
        self.message = message
        self.detail = detail

    def __str__(self) -> str:
        return f"{self.status}: {self.message}"


class NurtTimeoutError(NurtError, TimeoutError):
    """The execution was still running when the wait timed out."""


class NurtCancelledError(NurtError):
    """The execution ended because it was cancelled.

    ``response`` holds the response of the cancelled execution when the
    implementation returned one.
    """

    def __init__(
        self, status: str, message: str, *, detail: Any = None, response: Any = None
    ) -> None:
        super().__init__(status, message, detail=detail)
        self.response = response


class NurtLanguageError(Exception):
    """Error of the language, raised only when ``raise_on_error=True``.

    ``error`` is the ``error`` object of the report, ``code`` its ``E_*``
    code and ``response`` the complete :class:`nurt.Response`.
    """

    def __init__(self, response: Any) -> None:
        error: Mapping[str, Any] = response.error or {}
        self.response = response
        self.error = error
        self.code: str = str(error.get("code", ""))
        super().__init__(f"{self.code}: {error.get('message', '')}")


def error_for_status(
    code: int, message: str, *, detail: Any = None, response: Any = None
) -> NurtError:
    """Build the exception for a failed call with the given ``nurt_status`` code."""
    name = status_name(code)
    if name == UNKNOWN_STATUS:
        message = f"{message} (status {code})"
    if name == "NURT_E_TIMEOUT":
        return NurtTimeoutError(name, message, detail=detail)
    if name == "NURT_E_CANCELLED":
        return NurtCancelledError(name, message, detail=detail, response=response)
    return NurtError(name, message, detail=detail)

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
