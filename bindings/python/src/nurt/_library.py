# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""``library`` backend: ``libnurt`` loaded with :mod:`ctypes`.

This module is the only part of the package that depends on the C interface
declared in ``nurt.h``. ``ctypes`` releases the GIL for the duration of every
foreign call, so calls from several Python threads run in parallel.
"""

from __future__ import annotations

import atexit
import ctypes
import json
import logging
import os
import threading
from typing import Any, Callable, Dict, List, Mapping, Optional, Sequence, Set, Tuple

from ._backend import Backend, Diagnostics, EventSink, ExecutionHandle, Json
from ._errors import NurtCancelledError, NurtError, NurtTimeoutError, error_for_status
from ._response import failure_response, query_result

ABI_MAJOR = 1
"""Major version of the C interface required by this module."""

ABI_MINOR = 0
"""Lowest minor version of the C interface required by this module."""

ABI_VERSION = (ABI_MAJOR << 16) | ABI_MINOR
"""Value passed to ``nurt_abi_check``."""

WAIT_FOREVER = 0xFFFFFFFF
"""``NURT_WAIT_FOREVER``: timeout of ``nurt_exec_wait`` without limit."""

LONGEST_WAIT_MS = WAIT_FOREVER - 1
"""Longest finite timeout of ``nurt_exec_wait`` in milliseconds."""

STATUS_OK = 0
STATUS_TIMEOUT = 1
STATUS_CANCELLED = 2

VARIANTS: Dict[str, int] = {"a": 1, "b": 2, "c": 3}
"""Values of ``nurt_variant`` by notation letter."""

TRACE = 5
"""Level of the :mod:`logging` record for ``NURT_LOG_TRACE``."""

LOG_LEVELS: Dict[int, int] = {
    1: logging.ERROR,
    2: logging.WARNING,
    3: logging.INFO,
    4: logging.DEBUG,
    5: TRACE,
}
"""Levels of :mod:`logging` records by ``nurt_log_level``."""

logger = logging.getLogger("nurt")


class Buffer(ctypes.Structure):
    """``nurt_buf``: buffer returned by the library."""

    _fields_ = [
        ("ptr", ctypes.POINTER(ctypes.c_uint8)),
        ("len", ctypes.c_size_t),
        ("cap", ctypes.c_size_t),
    ]


EVENT_CALLBACK = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_char_p)
"""``nurt_event_cb``."""

LOG_CALLBACK = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_int, ctypes.c_char_p)
"""``nurt_log_cb``."""

_BUFFER = ctypes.POINTER(Buffer)
_HANDLE = ctypes.c_void_p
_HANDLE_OUT = ctypes.POINTER(ctypes.c_void_p)
_STATUS = ctypes.c_int
_TEXT = ctypes.c_char_p
_U32 = ctypes.c_uint32
_VARIANT = ctypes.c_int

SIGNATURES: Dict[str, Tuple[Any, Sequence[Any]]] = {
    "nurt_abi_version": (_U32, ()),
    "nurt_abi_check": (_STATUS, (_U32,)),
    "nurt_version": (_TEXT, ()),
    "nurt_info": (_STATUS, (_BUFFER,)),
    "nurt_engine_open": (_STATUS, (_TEXT, _HANDLE_OUT)),
    "nurt_engine_close": (None, (_HANDLE,)),
    "nurt_engine_config_get": (_STATUS, (_HANDLE, _BUFFER)),
    "nurt_parse": (_STATUS, (_HANDLE, _TEXT, _VARIANT, _BUFFER, _BUFFER)),
    "nurt_print": (_STATUS, (_HANDLE, _TEXT, _VARIANT, _BUFFER)),
    "nurt_correct": (_STATUS, (_HANDLE, _TEXT, _VARIANT, _BUFFER)),
    "nurt_check": (_STATUS, (_HANDLE, _TEXT, _BUFFER, _BUFFER)),
    "nurt_run_json": (_STATUS, (_HANDLE, _TEXT, _BUFFER)),
    "nurt_exec_start": (_STATUS, (_HANDLE, _TEXT, EVENT_CALLBACK, ctypes.c_void_p, _HANDLE_OUT)),
    "nurt_exec_wait": (_STATUS, (_HANDLE, _U32, _BUFFER)),
    "nurt_exec_cancel": (_STATUS, (_HANDLE,)),
    "nurt_exec_free": (None, (_HANDLE,)),
    "nurt_page": (_STATUS, (_HANDLE, _TEXT, _U32, _BUFFER)),
    "nurt_catalog": (_STATUS, (_HANDLE, _TEXT, _BUFFER)),
    "nurt_describe": (_STATUS, (_HANDLE, _TEXT, _BUFFER)),
    "nurt_help": (_STATUS, (_HANDLE, _TEXT, _BUFFER)),
    "nurt_schema": (_STATUS, (_TEXT, _BUFFER)),
    "nurt_history": (_STATUS, (_HANDLE, _TEXT, _BUFFER)),
    "nurt_space": (_STATUS, (_HANDLE, _TEXT, _BUFFER)),
    "nurt_session_end": (_STATUS, (_HANDLE, _TEXT)),
    "nurt_ext_load": (_STATUS, (_HANDLE, _TEXT)),
    "nurt_ext_list": (_STATUS, (_HANDLE, _BUFFER)),
    "nurt_buf_free": (None, (_BUFFER,)),
    "nurt_last_error": (_STATUS, (_HANDLE, _BUFFER)),
    "nurt_set_log_callback": (None, (LOG_CALLBACK, ctypes.c_void_p)),
}
"""Result and argument types of every function declared in ``nurt.h``."""


def _forward_log(_user: Optional[int], level: int, message: Optional[bytes]) -> None:
    try:
        text = (message or b"").decode("utf-8", "replace")
        logger.log(LOG_LEVELS.get(level, logging.DEBUG), "%s", text)
    except Exception:
        logger.debug("log message of libnurt dropped", exc_info=True)


_LOG_RECEIVER = LOG_CALLBACK(_forward_log)


class Library:
    """``libnurt`` loaded once per path, with checked interface version."""

    def __init__(self, path: str) -> None:
        self.path = path
        try:
            self.dll = ctypes.CDLL(path)
        except OSError as exc:
            raise NurtError("NURT_E_IO", f"cannot load {path}: {exc}") from exc
        for name, (restype, argtypes) in SIGNATURES.items():
            try:
                function = getattr(self.dll, name)
            except AttributeError as exc:
                raise NurtError(
                    "NURT_E_ABI", f"{path} does not export {name}; install a matching libnurt"
                ) from exc
            function.restype = restype
            function.argtypes = list(argtypes)
        if self.dll.nurt_abi_check(ABI_VERSION) != STATUS_OK:
            found = int(self.dll.nurt_abi_version())
            raise NurtError(
                "NURT_E_ABI",
                f"{path} implements interface {found >> 16}.{found & 0xFFFF}; "
                f"this package requires {ABI_MAJOR}.{ABI_MINOR} or a later minor version",
            )
        self.dll.nurt_set_log_callback(_LOG_RECEIVER, None)
        atexit.register(self.dll.nurt_set_log_callback, LOG_CALLBACK(), None)

    def call(self, name: str, args: Sequence[Any], outputs: int, engine: Optional[int]) -> List[bytes]:
        """Call ``name`` with ``args`` and ``outputs`` trailing output buffers.

        Every output buffer is released, also when the call fails. A status
        other than ``NURT_OK`` raises :class:`nurt.NurtError`.
        """
        buffers = [Buffer() for _ in range(outputs)]
        try:
            status = getattr(self.dll, name)(*args, *(ctypes.byref(b) for b in buffers))
            data = [_read(b) for b in buffers]
        finally:
            for buffer in buffers:
                self.dll.nurt_buf_free(ctypes.byref(buffer))
        if status != STATUS_OK:
            raise self.failure(name, status, engine)
        return data

    def failure(self, name: str, status: int, engine: Optional[int]) -> NurtError:
        """Exception for a failed call, with the record of ``nurt_last_error``."""
        buffer = Buffer()
        try:
            recorded = self.dll.nurt_last_error(engine, ctypes.byref(buffer))
            data = _read(buffer)
        finally:
            self.dll.nurt_buf_free(ctypes.byref(buffer))
        message = f"{name} failed"
        detail: Any = None
        if recorded == STATUS_OK and data:
            try:
                record = json.loads(data.decode("utf-8"))
            except ValueError:
                record = None
            if isinstance(record, dict):
                message = str(record.get("message") or message)
                detail = record.get("detail")
        return error_for_status(status, message, detail=detail)


_LIBRARIES: Dict[str, Library] = {}
_LIBRARIES_LOCK = threading.Lock()


def load(path: str) -> Library:
    """Return the library at ``path``, loading it on first use."""
    key = os.path.realpath(path)
    with _LIBRARIES_LOCK:
        library = _LIBRARIES.get(key)
        if library is None:
            library = Library(path)
            _LIBRARIES[key] = library
        return library


def _read(buffer: Buffer) -> bytes:
    if not buffer.len or not buffer.ptr:
        return b""
    return ctypes.string_at(buffer.ptr, buffer.len)


def _utf8(text: str) -> bytes:
    try:
        return text.encode("utf-8")
    except UnicodeEncodeError as exc:
        raise NurtError("NURT_E_ARG", f"the text is not valid Unicode: {exc.reason}") from exc


def _encode(value: Any) -> bytes:
    return _utf8(json.dumps(value, ensure_ascii=False, separators=(",", ":")))


def _text(value: Optional[str]) -> Optional[bytes]:
    """UTF-8 text for a ``const char *`` argument; a NUL character would end the string early."""
    if value is None:
        return None
    data = _utf8(value)
    if b"\0" in data:
        raise NurtError("NURT_E_ARG", "the text contains a NUL character")
    return data


def _decode(data: bytes, name: str) -> Any:
    try:
        return json.loads(data.decode("utf-8"))
    except ValueError as exc:
        raise NurtError("NURT_E_INTERNAL", f"{name} returned invalid JSON: {exc}") from exc


def _object(data: bytes, name: str) -> Json:
    value = _decode(data, name) if data else {}
    if not isinstance(value, dict):
        raise NurtError("NURT_E_INTERNAL", f"{name} returned JSON that is not an object")
    return value


def _failed(document: Json, kind: str) -> Json:
    """Response of the language error of a document ``{"error": ...}``; any other document is returned."""
    error = document.get("error")
    if isinstance(error, dict) and len(document) == 1:
        return failure_response(kind, error)
    return document


def _diagnostics(data: bytes, name: str) -> Tuple[Diagnostics, Optional[Json]]:
    """Diagnostics and language error of the second document of ``nurt_parse`` and ``nurt_check``."""
    value = _decode(data, name) if data else []
    error: Optional[Json] = None
    if isinstance(value, dict):
        found = value.get("error")
        error = found if isinstance(found, dict) else None
        value = value.get("diagnostics", [])
    items = [item for item in value if isinstance(item, dict)] if isinstance(value, list) else []
    return items, error


def _document(data: bytes, name: str) -> Optional[Json]:
    """First document of ``nurt_parse`` and ``nurt_check``: an object, or ``null`` after a language error."""
    value = _decode(data, name) if data else None
    if value is not None and not isinstance(value, dict):
        raise NurtError("NURT_E_INTERNAL", f"{name} returned JSON that is not an object")
    return value


class LibraryExecution(ExecutionHandle):
    """Execution started with ``nurt_exec_start``."""

    def __init__(self, backend: LibraryBackend, sink: EventSink) -> None:
        self._backend = backend
        self._sink = sink
        self._lock = threading.Lock()
        self._idle = threading.Condition(self._lock)
        self._active = 0
        self._ended = False
        self._pid = os.getpid()
        self._handle: Optional[int] = None
        self.callback: Optional[Callable[..., None]] = EVENT_CALLBACK(self._on_event)

    def attach(self, handle: Optional[int]) -> None:
        """Record the handle returned by ``nurt_exec_start``."""
        self._handle = handle

    def _on_event(self, _user: Optional[int], event_json: Optional[bytes]) -> None:
        try:
            event = json.loads((event_json or b"").decode("utf-8"))
        except ValueError:
            logger.warning("invalid event from libnurt dropped")
            return
        if not isinstance(event, dict):
            return
        try:
            self._sink(event)
        finally:
            if event.get("t") == "exec.finished":
                self._end()

    def _end(self) -> None:
        with self._lock:
            if self._ended:
                return
            self._ended = True
        self._sink(None)

    def _enter(self) -> int:
        """Handle for one call; :meth:`close` waits until the call has left."""
        with self._lock:
            if self._pid != os.getpid():
                raise NurtError("NURT_E_INTERNAL", "the execution belongs to the parent process")
            if self._handle is None:
                raise NurtError("NURT_E_ARG", "the execution is closed")
            self._active += 1
            return self._handle

    def _leave(self) -> None:
        with self._idle:
            self._active -= 1
            if self._active == 0:
                self._idle.notify_all()

    def wait(self, timeout: Optional[float]) -> Json:
        if timeout is None or timeout == float("inf"):
            milliseconds = WAIT_FOREVER
        elif timeout != timeout:
            raise NurtError("NURT_E_ARG", "the timeout is not a number")
        else:
            milliseconds = int(min(max(timeout * 1000, 0.0), float(LONGEST_WAIT_MS)))
        handle = self._enter()
        try:
            return self._wait(handle, milliseconds, timeout)
        finally:
            self._leave()

    def _wait(self, handle: int, milliseconds: int, timeout: Optional[float]) -> Json:
        library = self._backend.library
        buffer = Buffer()
        try:
            status = library.dll.nurt_exec_wait(handle, milliseconds, ctypes.byref(buffer))
            data = _read(buffer)
        finally:
            library.dll.nurt_buf_free(ctypes.byref(buffer))
        if status == STATUS_OK:
            return _object(data, "nurt_exec_wait")
        if status == STATUS_TIMEOUT:
            raise NurtTimeoutError("NURT_E_TIMEOUT", f"the execution is still running after {timeout} s")
        if status == STATUS_CANCELLED:
            response = _object(data, "nurt_exec_wait") if data else None
            raise NurtCancelledError("NURT_E_CANCELLED", "the execution was cancelled", response=response)
        raise library.failure("nurt_exec_wait", status, None)

    def cancel(self) -> None:
        handle = self._enter()
        try:
            library = self._backend.library
            status = library.dll.nurt_exec_cancel(handle)
            if status != STATUS_OK:
                raise library.failure("nurt_exec_cancel", status, None)
        finally:
            self._leave()

    def close(self) -> None:
        with self._lock:
            handle, self._handle = self._handle, None
        if handle is not None and self._pid == os.getpid():
            library = self._backend.library
            # A call blocked in nurt_exec_wait must return before the handle is released.
            library.dll.nurt_exec_cancel(handle)
            with self._idle:
                while self._active:
                    self._idle.wait()
            library.dll.nurt_exec_free(handle)
        self.callback = None
        self._backend.forget(self)
        self._end()


class LibraryBackend(Backend):
    """Engine of ``libnurt`` in the calling process.

    ``session`` and ``language`` are written into the options of every
    request that does not set them, and ``language`` into every description
    query without a language.
    """

    name = "library"

    def __init__(
        self, path: str, config: Optional[str], *, session: Optional[str] = None, language: Optional[str] = None
    ) -> None:
        self.library = load(path)
        self._config = _text(config)
        self._defaults: Dict[str, str] = {}
        if session is not None:
            self._defaults["session"] = session
        if language is not None:
            self._defaults["language"] = language
        self._lock = threading.Lock()
        self._idle = threading.Condition(self._lock)
        self._active = 0
        self._executions: Set[LibraryExecution] = set()
        self._closed = False
        self._engine: Optional[int] = None
        self._pid = 0
        self._open()

    def _open(self) -> None:
        handle = ctypes.c_void_p()
        status = self.library.dll.nurt_engine_open(self._config, ctypes.byref(handle))
        if status != STATUS_OK:
            raise self.library.failure("nurt_engine_open", status, None)
        self._engine = handle.value
        self._pid = os.getpid()

    def _enter(self) -> Optional[int]:
        """Engine handle for one call; :meth:`close` waits until the call has left."""
        with self._lock:
            if self._closed:
                raise NurtError("NURT_E_ARG", "the engine is closed")
            if self._pid != os.getpid():
                self._executions.clear()
                self._active = 0
                self._open()
            self._active += 1
            return self._engine

    def _leave(self) -> None:
        with self._idle:
            self._active -= 1
            if self._active == 0:
                self._idle.notify_all()

    def _call(self, name: str, args: Sequence[Any], outputs: int) -> List[bytes]:
        engine = self._enter()
        try:
            return self.library.call(name, (engine, *args), outputs, engine)
        finally:
            self._leave()

    def forget(self, execution: LibraryExecution) -> None:
        """Remove a closed execution from the set released by :meth:`close`."""
        with self._lock:
            self._executions.discard(execution)

    def _request(self, request: Mapping[str, Any]) -> bytes:
        envelope = dict(request)
        options = dict(envelope.get("options") or {})
        for name, value in self._defaults.items():
            options.setdefault(name, value)
        if options:
            envelope["options"] = options
        return _encode(envelope)

    def run(self, request: Mapping[str, Any]) -> Json:
        (data,) = self._call("nurt_run_json", (self._request(request),), 1)
        return _object(data, "nurt_run_json")

    def start(self, request: Mapping[str, Any], sink: EventSink) -> ExecutionHandle:
        text = self._request(request)
        engine = self._enter()
        try:
            execution = LibraryExecution(self, sink)
            handle = ctypes.c_void_p()
            status = self.library.dll.nurt_exec_start(
                engine, text, execution.callback, None, ctypes.byref(handle)
            )
            if status != STATUS_OK:
                raise self.library.failure("nurt_exec_start", status, engine)
            execution.attach(handle.value)
            with self._lock:
                self._executions.add(execution)
        finally:
            self._leave()
        return execution

    def check(self, request: Mapping[str, Any]) -> Json:
        bound, diagnostics = self._call("nurt_check", (self._request(request),), 2)
        items, error = _diagnostics(diagnostics, "nurt_check")
        if error is not None:
            return failure_response("check", error)
        result = dict(_document(bound, "nurt_check") or {})
        result["diagnostics"] = items
        return result

    def parse(self, program: str, variant: str) -> Json:
        ast, diagnostics = self._call("nurt_parse", (_text(program), VARIANTS[variant]), 2)
        items, error = _diagnostics(diagnostics, "nurt_parse")
        if error is not None:
            return failure_response("parse", error)
        return {"ast": _document(ast, "nurt_parse"), "diagnostics": items}

    def print(self, ast: Mapping[str, Any], variant: str) -> str:
        (text,) = self._call("nurt_print", (_encode(ast), VARIANTS[variant]), 1)
        return text.decode("utf-8")

    def correct(self, program: str, variant: str) -> Json:
        (data,) = self._call("nurt_correct", (_text(program), VARIANTS[variant]), 1)
        return _failed(_object(data, "nurt_correct"), "correct")

    def page(self, cursor: str, limit: Optional[int]) -> Json:
        (data,) = self._call("nurt_page", (_text(cursor), limit or 0), 1)
        return _object(data, "nurt_page")

    def describe(self, what: Mapping[str, Any]) -> str:
        query = dict(what)
        if "language" in self._defaults:
            query.setdefault("language", self._defaults["language"])
        (text,) = self._call("nurt_describe", (_encode(query),), 1)
        return text.decode("utf-8")

    def help(self, topic: Optional[str]) -> str:
        (text,) = self._call("nurt_help", (_text(topic),), 1)
        return text.decode("utf-8")

    def catalog(self, what: Optional[Mapping[str, Any]]) -> Json:
        selector = None if what is None else _encode(what)
        (data,) = self._call("nurt_catalog", (selector,), 1)
        return query_result(_failed(_object(data, "nurt_catalog"), "catalog"))

    def schema(self, which: str) -> Json:
        (data,) = self.library.call("nurt_schema", (_text(which),), 1, None)
        return _object(data, "nurt_schema")

    def history(self, query: Any) -> Json:
        # A text is the JSON text of the query; an encoded text would be taken for no query at all.
        selector = None if query is None else _text(query) if isinstance(query, str) else _encode(query)
        (data,) = self._call("nurt_history", (selector,), 1)
        return query_result(_failed(_object(data, "nurt_history"), "history"))

    def space(self, volume: Optional[str]) -> Json:
        (data,) = self._call("nurt_space", (_text(volume),), 1)
        return query_result(_failed(_object(data, "nurt_space"), "space"))

    def info(self) -> Json:
        (data,) = self.library.call("nurt_info", (), 1, None)
        return _object(data, "nurt_info")

    def close(self) -> None:
        with self._lock:
            if self._closed:
                return
            self._closed = True
            executions = list(self._executions)
            engine = self._engine
        for execution in executions:
            execution.close()
        owned = self._pid == os.getpid()
        with self._idle:
            # The engine must not be released while a call uses it; after fork() no thread of this process does.
            while owned and self._active:
                self._idle.wait()
            self._engine = None
        if engine is not None and owned:
            self.library.dll.nurt_engine_close(engine)

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass
