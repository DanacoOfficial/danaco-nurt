# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Synchronous client."""

from __future__ import annotations

import json
import queue
import threading
from typing import Any, Callable, Dict, Iterator, Mapping, Optional, Union

from ._backend import Backend, ExecutionHandle
from ._errors import NurtCancelledError, NurtError, NurtLanguageError
from ._locate import StrPath, find_executable, find_library
from ._response import CheckResult, CorrectResult, ParseResult, Response
from .types import REQUEST_OPTIONS

ENVELOPE_VERSION = "1.0"
"""Version of the request envelope ``nurt-request/1`` written by the client."""

BACKENDS = ("auto", "library", "process")
VARIANTS = ("a", "b", "c")

EVENT_DRAIN_TIMEOUT = 5.0
"""Longest wait in seconds for the delivery of the remaining events after the response."""

_ENVELOPE_FIELDS = ("variant", "cursor", "accept")
_ACCEPTED_OPTIONS = frozenset(REQUEST_OPTIONS) | frozenset(_ENVELOPE_FIELDS)
_END = object()

EventCallback = Callable[[Dict[str, Any]], None]
Config = Union[None, str, Mapping[str, Any]]
HistoryQuery = Union[None, str, Mapping[str, Any]]


class Execution:
    """Asynchronous execution started by :meth:`Nurt.start`.

    Events are delivered to ``on_event`` from a background thread, one at a
    time and in order, or, without ``on_event``, by iterating the execution.
    Use the execution as a context manager or call :meth:`close`.
    """

    def __init__(
        self,
        on_event: Optional[EventCallback],
        raise_on_error: bool,
        on_end: Optional[Callable[[], None]] = None,
    ) -> None:
        self._on_event = on_event
        self._on_end = on_end
        self._raise_on_error = raise_on_error
        self._events: "queue.Queue[Any]" = queue.Queue()
        self._ended = threading.Event()
        self._callback_error: Optional[BaseException] = None
        self._handle: Optional[ExecutionHandle] = None

    def _attach(self, handle: ExecutionHandle) -> None:
        self._handle = handle

    def _sink(self, event: Optional[Dict[str, Any]]) -> None:
        if event is None:
            self._events.put(_END)
            if self._on_end is not None:
                self._on_end()
            self._ended.set()
            return
        if self._on_event is None:
            self._events.put(event)
            return
        try:
            self._on_event(event)
        except Exception as exc:
            if self._callback_error is None:
                self._callback_error = exc

    def _live(self) -> ExecutionHandle:
        if self._handle is None:
            raise NurtError("NURT_E_ARG", "the execution was not started")
        return self._handle

    def wait(self, timeout: Optional[float] = None) -> Response:
        """Wait for the response; ``None`` waits without limit.

        A cancelled execution returns its response, whose ``error`` has the
        code ``E_CANCELLED``. Raises :class:`nurt.NurtTimeoutError` when the
        execution is still running after ``timeout`` seconds,
        :class:`nurt.NurtCancelledError` when it was cancelled without a
        response, and the first exception raised by ``on_event``. Events are
        delivered to ``on_event`` before the response is returned.
        """
        try:
            raw = self._live().wait(timeout)
        except NurtCancelledError as exc:
            self._ended.wait(EVENT_DRAIN_TIMEOUT)
            if not isinstance(exc.response, Mapping):
                raise
            raw = dict(exc.response)
        self._ended.wait(EVENT_DRAIN_TIMEOUT)
        if self._callback_error is not None:
            raise self._callback_error
        response = Response(raw)
        if self._raise_on_error and not response.ok:
            raise NurtLanguageError(response)
        return response

    def cancel(self) -> None:
        """Request cancellation; idempotent. Committed changes stay in the journal."""
        self._live().cancel()

    def close(self) -> None:
        """Release the execution, cancelling it first when it is running."""
        if self._handle is not None:
            self._handle.close()

    def __iter__(self) -> Iterator[Dict[str, Any]]:
        if self._on_event is not None:
            raise TypeError("events of this execution are delivered to on_event")
        while True:
            event = self._events.get()
            if event is _END:
                self._events.put(_END)
                return
            yield event

    def __enter__(self) -> Execution:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()


class Nurt:
    """Client of the Danaco Nurt engine.

    ``backend`` selects ``"library"`` (``libnurt`` in this process),
    ``"process"`` (a child process ``nurt serve --stdio``) or ``"auto"``
    (the library when it is available and compatible, otherwise the
    process). ``executable`` and ``library`` override the search for the
    artifacts; ``config`` is a ``nurt-config/1`` object or its JSON text;
    ``session`` and ``language`` apply to every request that does not set
    them. The client is thread-safe.

    Language errors are returned in :class:`Response` (``ok`` is ``False``);
    failures of the call raise :class:`NurtError`.
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
        if backend not in BACKENDS:
            raise ValueError(f"unknown backend {backend!r}; use auto, library or process")
        config_object = _config_object(config)
        self._backend = _open_backend(backend, executable, library, config_object, session, language)

    @property
    def backend(self) -> str:
        """The backend in use: ``"library"`` or ``"process"``."""
        return self._backend.name

    def run(
        self,
        program: Optional[str] = None,
        *,
        ast: Any = None,
        raise_on_error: bool = False,
        **options: Any,
    ) -> Response:
        """Run a program, given as text or as a syntax tree, and return the response.

        ``options`` are the options of ``nurt-request/1`` (``preview``,
        ``base``, ``limit``, ``format``, ``timeout``, ...) and the envelope
        fields ``variant``, ``cursor`` and ``accept``. With
        ``raise_on_error=True`` a language error raises
        :class:`NurtLanguageError`.
        """
        response = Response(self._backend.run(_envelope(program, ast, options)))
        if raise_on_error and not response.ok:
            raise NurtLanguageError(response)
        return response

    def start(
        self,
        program: Optional[str] = None,
        *,
        ast: Any = None,
        on_event: Optional[EventCallback] = None,
        raise_on_error: bool = False,
        **options: Any,
    ) -> Execution:
        """Start a program and return the :class:`Execution` without waiting."""
        return self._start(program, ast, on_event, raise_on_error, None, options)

    def _start(
        self,
        program: Optional[str],
        ast: Any,
        on_event: Optional[EventCallback],
        raise_on_error: bool,
        on_end: Optional[Callable[[], None]],
        options: Dict[str, Any],
    ) -> Execution:
        request = _envelope(program, ast, options)
        execution = Execution(on_event, raise_on_error, on_end)
        execution._attach(self._backend.start(request, execution._sink))
        return execution

    def check(self, program: Optional[str] = None, *, ast: Any = None, **options: Any) -> CheckResult:
        """Bind a program and classify its effect without running it."""
        return CheckResult(self._backend.check(_envelope(program, ast, options)))

    def parse(self, program: str, variant: str = "a") -> ParseResult:
        """Parse a program into a syntax tree ``nurt-ast/1``."""
        return ParseResult(self._backend.parse(program, _variant(variant)))

    def print(self, ast: Any, variant: str = "a") -> str:
        """Print a syntax tree in the canonical text of a notation variant."""
        tree = json.loads(ast) if isinstance(ast, str) else ast
        return self._backend.print(tree, _variant(variant))

    def correct(self, program: str, variant: str = "a") -> CorrectResult:
        """Correct the written form of a program without running it."""
        return CorrectResult(self._backend.correct(program, _variant(variant)))

    def page(self, cursor: str, limit: Optional[int] = None, *, raise_on_error: bool = False) -> Response:
        """Return a further page of a paged result."""
        response = Response(self._backend.page(cursor, limit))
        if raise_on_error and not response.ok:
            raise NurtLanguageError(response)
        return response

    def describe(self, layer: str = "core", name: Optional[str] = None, language: Optional[str] = None) -> str:
        """Return the description of the language for tool integrations.

        ``language`` defaults to the language of the client.
        """
        what: Dict[str, Any] = {"layer": layer}
        if name is not None:
            what["name"] = name
        if language is not None:
            what["language"] = language
        return self._backend.describe(what)

    def help(self, topic: Optional[str] = None) -> str:
        """Return built-in help on a topic, or the index of topics."""
        return self._backend.help(topic)

    def catalog(self, what: Optional[Mapping[str, Any]] = None) -> Dict[str, Any]:
        """Return the command catalog, or the group or command selected by ``what``."""
        return self._backend.catalog(what)

    def schema(self, which: str) -> Dict[str, Any]:
        """Return a published JSON schema: ``ast``, ``bound``, ``report``, ``request``, ..."""
        return self._backend.schema(which)

    def history(self, query: HistoryQuery = None) -> Dict[str, Any]:
        """Return journal history; ``query`` is an object with the parameters of ``history`` or a query text."""
        return self._backend.history(query if query is None or isinstance(query, str) else dict(query))

    def space(self, volume: Optional[str] = None) -> Dict[str, Any]:
        """Return free space and journal usage of a volume, or of all volumes."""
        return self._backend.space(volume)

    def info(self) -> Dict[str, Any]:
        """Return the version, interface versions and features of the implementation."""
        return self._backend.info()

    def close(self) -> None:
        """Release the engine; running executions are cancelled. Idempotent."""
        self._backend.close()

    def __enter__(self) -> Nurt:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def __repr__(self) -> str:
        return f"Nurt(backend={self.backend!r})"


def _envelope(program: Optional[str], ast: Any, options: Dict[str, Any]) -> Dict[str, Any]:
    """Request envelope ``nurt-request/1`` with the options given by the caller only.

    The defaults of the client (``session``, ``language``) are applied by the
    backend: ``nurt serve`` receives them in ``initialize``.
    """
    unknown = sorted(set(options) - _ACCEPTED_OPTIONS)
    if unknown:
        raise TypeError(f"unknown option {unknown[0]!r}")
    request: Dict[str, Any] = {"nurt": ENVELOPE_VERSION}
    if program is not None:
        request["program"] = program
    if ast is not None:
        request["ast"] = json.loads(ast) if isinstance(ast, str) else dict(ast)
    for field in _ENVELOPE_FIELDS:
        value = options.pop(field, None)
        if value is not None:
            request[field] = value
    call_options = {key: value for key, value in options.items() if value is not None}
    if call_options:
        request["options"] = call_options
    return request


def _variant(variant: str) -> str:
    if variant not in VARIANTS:
        raise NurtError("NURT_E_ARG", f"unknown notation variant {variant!r}; use a, b or c")
    return variant


def _config_object(config: Config) -> Optional[Dict[str, Any]]:
    if config is None:
        return None
    value = json.loads(config) if isinstance(config, str) else dict(config)
    if not isinstance(value, dict):
        raise NurtError("NURT_E_ARG", "config must be a JSON object")
    return value


def _open_backend(
    backend: str,
    executable: Optional[StrPath],
    library: Optional[StrPath],
    config: Optional[Dict[str, Any]],
    session: Optional[str],
    language: str,
) -> Backend:
    if backend == "library":
        return _library_backend(library, config, session, language)
    if backend == "process":
        return _process_backend(executable, config, session, language)
    try:
        return _library_backend(library, config, session, language)
    except NurtError as library_error:
        try:
            return _process_backend(executable, config, session, language)
        except NurtError as process_error:
            raise NurtError(
                process_error.status,
                f"{process_error.message}; library backend: {library_error.message}",
                detail={"process": process_error.detail, "library": library_error.detail},
            ) from process_error


def _library_backend(
    library: Optional[StrPath],
    config: Optional[Dict[str, Any]],
    session: Optional[str],
    language: str,
) -> Backend:
    # The ctypes module is imported only for the library backend, so the
    # process backend works on interpreters built without it.
    try:
        from ._library import LibraryBackend
    except ImportError as exc:
        raise NurtError("NURT_E_IO", f"the library backend needs ctypes: {exc}") from exc

    path = find_library(library)
    text = None if config is None else json.dumps(config, ensure_ascii=False)
    return LibraryBackend(path, text, session=session, language=language)


def _process_backend(
    executable: Optional[StrPath],
    config: Optional[Dict[str, Any]],
    session: Optional[str],
    language: str,
) -> Backend:
    from ._process import ProcessBackend

    return ProcessBackend(find_executable(executable), config=config, session=session, language=language)
