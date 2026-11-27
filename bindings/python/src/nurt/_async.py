# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bindings/python/src/nurt/_async.py
# Responsibility:   Script _async.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Client for :mod:`asyncio`.

Calls run in the default executor of the event loop, so they do not block
the loop; events of executions are delivered in the loop thread.
"""

from __future__ import annotations

import asyncio
import functools
from typing import Any, AsyncIterator, Callable, Dict, Mapping, Optional, TypeVar

from ._client import Config, EventCallback, Execution, HistoryQuery, Nurt
from ._errors import NurtError
from ._locate import StrPath
from ._response import CheckResult, CorrectResult, ParseResult, Response

T = TypeVar("T")

_END = object()


class AsyncExecution:
    """Execution started by :meth:`AsyncNurt.start`.

    Events are delivered to ``on_event`` in the event loop thread or, without
    ``on_event``, by ``async for``.
    """

    def __init__(self, loop: asyncio.AbstractEventLoop, on_event: Optional[EventCallback]) -> None:
        self._loop = loop
        self._on_event = on_event
        self._events: "asyncio.Queue[Any]" = asyncio.Queue()
        self._execution: Optional[Execution] = None

    def _attach(self, execution: Execution) -> None:
        self._execution = execution

    def _from_thread(self, event: Dict[str, Any]) -> None:
        self._post(self._deliver, event)

    def _end_from_thread(self) -> None:
        self._post(self._events.put_nowait, _END)

    def _post(self, function: Callable[..., None], *args: Any) -> None:
        try:
            self._loop.call_soon_threadsafe(function, *args)
        except RuntimeError:
            return

    def _deliver(self, event: Dict[str, Any]) -> None:
        if self._on_event is None:
            self._events.put_nowait(event)
        else:
            self._on_event(event)

    def _live(self) -> Execution:
        if self._execution is None:
            raise NurtError("NURT_E_ARG", "the execution was not started")
        return self._execution

    async def wait(self, timeout: Optional[float] = None) -> Response:
        """Wait for the response; see :meth:`nurt.Execution.wait`."""
        return await _in_executor(self._live().wait, timeout)

    async def cancel(self) -> None:
        """Request cancellation; idempotent."""
        await _in_executor(self._live().cancel)

    async def close(self) -> None:
        """Release the execution, cancelling it first when it is running."""
        await _in_executor(self._live().close)

    def __aiter__(self) -> AsyncIterator[Dict[str, Any]]:
        if self._on_event is not None:
            raise TypeError("events of this execution are delivered to on_event")
        return self._iterate()

    async def _iterate(self) -> AsyncIterator[Dict[str, Any]]:
        while True:
            event = await self._events.get()
            if event is _END:
                self._events.put_nowait(_END)
                return
            yield event

    async def __aenter__(self) -> AsyncExecution:
        return self

    async def __aexit__(self, *_: object) -> None:
        await self.close()


class AsyncNurt:
    """Client of the Danaco Nurt engine for :mod:`asyncio`.

    The arguments and methods are those of :class:`nurt.Nurt`; every method
    is a coroutine.
    """

    def __init__(
        self,
        backend: str = "auto",
        executable: Optional[StrPath] = None,
        library: Optional[StrPath] = None,
        config: Config = None,
        session: Optional[str] = None,
        language: str = "en",
    ) -> None:
        self._nurt = Nurt(backend, executable, library, config, session, language)

    @property
    def backend(self) -> str:
        """The backend in use: ``"library"`` or ``"process"``."""
        return self._nurt.backend

    async def run(
        self, program: Optional[str] = None, *, ast: Any = None, raise_on_error: bool = False, **options: Any
    ) -> Response:
        """Run a program; see :meth:`nurt.Nurt.run`."""
        return await _in_executor(
            functools.partial(self._nurt.run, program, ast=ast, raise_on_error=raise_on_error, **options)
        )

    async def start(
        self,
        program: Optional[str] = None,
        *,
        ast: Any = None,
        on_event: Optional[EventCallback] = None,
        raise_on_error: bool = False,
        **options: Any,
    ) -> AsyncExecution:
        """Start a program; see :meth:`nurt.Nurt.start`."""
        execution = AsyncExecution(asyncio.get_running_loop(), on_event)
        started = await _in_executor(
            self._nurt._start,
            program,
            ast,
            execution._from_thread,
            raise_on_error,
            execution._end_from_thread,
            options,
        )
        execution._attach(started)
        return execution

    async def check(self, program: Optional[str] = None, *, ast: Any = None, **options: Any) -> CheckResult:
        """Bind a program without running it; see :meth:`nurt.Nurt.check`."""
        return await _in_executor(functools.partial(self._nurt.check, program, ast=ast, **options))

    async def parse(self, program: str, variant: str = "a") -> ParseResult:
        """Parse a program; see :meth:`nurt.Nurt.parse`."""
        return await _in_executor(self._nurt.parse, program, variant)

    async def print(self, ast: Any, variant: str = "a") -> str:
        """Print a syntax tree; see :meth:`nurt.Nurt.print`."""
        return await _in_executor(self._nurt.print, ast, variant)

    async def correct(self, program: str, variant: str = "a") -> CorrectResult:
        """Correct a program; see :meth:`nurt.Nurt.correct`."""
        return await _in_executor(self._nurt.correct, program, variant)

    async def page(self, cursor: str, limit: Optional[int] = None, *, raise_on_error: bool = False) -> Response:
        """Return a further page; see :meth:`nurt.Nurt.page`."""
        return await _in_executor(functools.partial(self._nurt.page, cursor, limit, raise_on_error=raise_on_error))

    async def describe(self, layer: str = "core", name: Optional[str] = None, language: Optional[str] = None) -> str:
        """Return the description of the language; see :meth:`nurt.Nurt.describe`."""
        return await _in_executor(self._nurt.describe, layer, name, language)

    async def help(self, topic: Optional[str] = None) -> str:
        """Return built-in help; see :meth:`nurt.Nurt.help`."""
        return await _in_executor(self._nurt.help, topic)

    async def catalog(self, what: Optional[Mapping[str, Any]] = None) -> Dict[str, Any]:
        """Return the command catalog; see :meth:`nurt.Nurt.catalog`."""
        return await _in_executor(self._nurt.catalog, what)

    async def schema(self, which: str) -> Dict[str, Any]:
        """Return a published JSON schema; see :meth:`nurt.Nurt.schema`."""
        return await _in_executor(self._nurt.schema, which)

    async def history(self, query: HistoryQuery = None) -> Dict[str, Any]:
        """Return journal history; see :meth:`nurt.Nurt.history`."""
        return await _in_executor(self._nurt.history, query)

    async def space(self, volume: Optional[str] = None) -> Dict[str, Any]:
        """Return free space and journal usage; see :meth:`nurt.Nurt.space`."""
        return await _in_executor(self._nurt.space, volume)

    async def info(self) -> Dict[str, Any]:
        """Return information about the implementation; see :meth:`nurt.Nurt.info`."""
        return await _in_executor(self._nurt.info)

    async def close(self) -> None:
        """Release the engine; idempotent."""
        await _in_executor(self._nurt.close)

    async def __aenter__(self) -> AsyncNurt:
        return self

    async def __aexit__(self, *_: object) -> None:
        await self.close()

    def __repr__(self) -> str:
        return f"AsyncNurt(backend={self.backend!r})"


async def _in_executor(function: Callable[..., T], *args: Any) -> T:
    return await asyncio.get_running_loop().run_in_executor(None, function, *args)

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
