# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Interface shared by the ``library`` and ``process`` backends.

Every method takes and returns decoded JSON. Backends are thread-safe.
"""

from __future__ import annotations

import abc
from typing import Any, Callable, Dict, List, Mapping, Optional

EventSink = Callable[[Optional[Dict[str, Any]]], None]
"""Receiver of the events of one execution.

Called from one backend thread per execution, one event at a time, in the
order of ``seq``; ``None`` marks the end of the events.
"""

Json = Dict[str, Any]
Diagnostics = List[Dict[str, Any]]


class ExecutionHandle(abc.ABC):
    """Execution started by :meth:`Backend.start`."""

    @abc.abstractmethod
    def wait(self, timeout: Optional[float]) -> Json:
        """Wait for the response; ``None`` waits without limit.

        Raises :class:`nurt.NurtTimeoutError` when the execution is still
        running and :class:`nurt.NurtCancelledError` after cancellation.
        """

    @abc.abstractmethod
    def cancel(self) -> None:
        """Request cancellation; idempotent."""

    @abc.abstractmethod
    def close(self) -> None:
        """Release the execution, cancelling it first when it is running."""


class Backend(abc.ABC):
    """Access to one engine, either in-process or through ``nurt serve``."""

    name: str
    """``"library"`` or ``"process"``."""

    @abc.abstractmethod
    def run(self, request: Mapping[str, Any]) -> Json:
        """Run a request envelope and return the response."""

    @abc.abstractmethod
    def start(self, request: Mapping[str, Any], sink: EventSink) -> ExecutionHandle:
        """Start a request envelope; events go to ``sink``."""

    @abc.abstractmethod
    def check(self, request: Mapping[str, Any]) -> Json:
        """Bind and classify a request envelope without running it.

        The result holds the members of ``nurt-check/1``, ``diagnostics`` and,
        after a language error, ``error``.
        """

    @abc.abstractmethod
    def parse(self, program: str, variant: str) -> Json:
        """Parse a program; the result holds ``ast``, ``diagnostics`` and, after a language error, ``error``."""

    @abc.abstractmethod
    def print(self, ast: Mapping[str, Any], variant: str) -> str:
        """Print a syntax tree in the canonical text of a notation."""

    @abc.abstractmethod
    def correct(self, program: str, variant: str) -> Json:
        """Correct the written form of a program."""

    @abc.abstractmethod
    def page(self, cursor: str, limit: Optional[int]) -> Json:
        """Return a further page of a paged result."""

    @abc.abstractmethod
    def describe(self, what: Mapping[str, Any]) -> str:
        """Return the description of the language for tool integrations."""

    @abc.abstractmethod
    def help(self, topic: Optional[str]) -> str:
        """Return built-in help on a topic."""

    @abc.abstractmethod
    def catalog(self, what: Optional[Mapping[str, Any]]) -> Json:
        """Return the command catalog or a part of it."""

    @abc.abstractmethod
    def schema(self, which: str) -> Json:
        """Return a published JSON schema by name."""

    @abc.abstractmethod
    def history(self, query: Any) -> Json:
        """Return journal history; ``query`` is ``None``, an object or a query text."""

    @abc.abstractmethod
    def space(self, volume: Optional[str]) -> Json:
        """Return free space and journal usage."""

    @abc.abstractmethod
    def info(self) -> Json:
        """Return information about the implementation."""

    @abc.abstractmethod
    def close(self) -> None:
        """Release the engine; idempotent."""
