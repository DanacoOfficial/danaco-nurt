# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""``process`` backend: a child process ``nurt serve --stdio``.

Messages are JSON-RPC 2.0 objects of the protocol ``nurt-rpc/1``, one per
line (NDJSON). Several requests may be in flight; responses are matched by
``id``. A reader thread dispatches responses and events. When the child
process exits, requests in flight fail with ``NURT_E_INTERNAL`` and the next
call starts a new process.
"""

from __future__ import annotations

import collections
import itertools
import json
import logging
import os
import queue
import subprocess
import threading
from typing import IO, Any, Deque, Dict, List, Mapping, Optional, Sequence, Tuple, Union

from ._backend import Backend, Diagnostics, EventSink, ExecutionHandle, Json
from ._errors import NurtError, NurtTimeoutError, error_for_status
from ._version import __version__

PROTOCOL = "nurt-rpc/1"
"""Protocol requested in ``initialize``."""

CLIENT_NAME = "nurt-python"
"""Client name sent in ``initialize``."""

EVENT_METHOD = "nurt/event"
"""Method of event notifications."""

APPLICATION_ERROR_BASE = -32000
"""JSON-RPC error code of ``NURT_OK``; a call failure with status ``s`` has code ``-32000 - s``."""

APPLICATION_ERROR_LAST = -32099
"""Last JSON-RPC error code of the application range."""

INVALID_PARAMS = -32602
"""JSON-RPC error code of invalid parameters, reported as ``NURT_E_ARG``."""

STATUS_ARG = 3
STATUS_INTERNAL = 6

SHUTDOWN_TIMEOUT = 5.0
"""Seconds allowed for ``shutdown`` and for the process to exit afterwards."""

UNLIMITED_METHODS = frozenset({"initialize", "cancel", "shutdown"})
"""Methods sent without taking one of the ``max_inflight`` slots."""

STDERR_TAIL_LINES = 20
"""Lines of standard error kept for the message about an exited process."""

logger = logging.getLogger("nurt")

_END = object()

Outcome = Union[Dict[str, Any], NurtError]


def _encode(message: Mapping[str, Any]) -> bytes:
    return json.dumps(message, ensure_ascii=False, separators=(",", ":")).encode("utf-8") + b"\n"


def _failure(message: Mapping[str, Any]) -> NurtError:
    error = message.get("error")
    if not isinstance(error, dict):
        return NurtError("NURT_E_INTERNAL", "invalid error object from nurt serve")
    code = error.get("code")
    data = error.get("data")
    if isinstance(code, int) and APPLICATION_ERROR_LAST <= code <= APPLICATION_ERROR_BASE:
        status = APPLICATION_ERROR_BASE - code
    elif code == INVALID_PARAMS:
        status = STATUS_ARG
    else:
        status = STATUS_INTERNAL
    response = data.get("response") if isinstance(data, dict) else None
    text = str(error.get("message") or f"nurt serve returned error {code}")
    return error_for_status(status, text, detail=data, response=response)


def _outcome(outcome: Optional[Outcome]) -> Any:
    if isinstance(outcome, NurtError):
        raise outcome
    if outcome is None:
        raise NurtError("NURT_E_INTERNAL", "no response from nurt serve")
    if "error" in outcome:
        raise _failure(outcome)
    return outcome.get("result")


class _Pending:
    __slots__ = ("done", "outcome", "execution", "slot")

    def __init__(self, execution: Optional[ProcessExecution], slot: bool) -> None:
        self.done = threading.Event()
        self.outcome: Optional[Outcome] = None
        self.execution = execution
        self.slot = slot


class _Connection:
    """One running ``nurt serve --stdio`` process."""

    def __init__(self, argv: Sequence[str], initialize: Mapping[str, Any]) -> None:
        creationflags = getattr(subprocess, "CREATE_NO_WINDOW", 0) if os.name == "nt" else 0
        try:
            self.process = subprocess.Popen(
                list(argv),
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                creationflags=creationflags,
            )
        except OSError as exc:
            raise NurtError("NURT_E_IO", f"cannot start {argv[0]}: {exc}") from exc
        self.pid = os.getpid()
        self.dead = False
        self._stdin: IO[bytes] = self.process.stdin  # type: ignore[assignment]
        self._lock = threading.Lock()
        self._write_lock = threading.Lock()
        self._pending: Dict[int, _Pending] = {}
        self._ids = itertools.count(1)
        self._stderr: Deque[str] = collections.deque(maxlen=STDERR_TAIL_LINES)
        self._slots: Optional[threading.BoundedSemaphore] = None
        self._stderr_reader = threading.Thread(
            target=self._read_stderr, name="nurt-serve-stderr", daemon=True
        )
        self._stderr_reader.start()
        threading.Thread(target=self._read_stdout, name="nurt-serve-reader", daemon=True).start()
        try:
            self._accept(self.call("initialize", initialize))
        except NurtError:
            self.terminate()
            raise

    def _accept(self, result: Any) -> None:
        if not isinstance(result, dict):
            raise NurtError("NURT_E_INTERNAL", "invalid result of initialize")
        protocol = result.get("protocol")
        if protocol != PROTOCOL:
            raise NurtError(
                "NURT_E_ABI",
                f"nurt serve speaks {protocol}; this package requires {PROTOCOL}",
            )
        limits = result.get("limits")
        inflight = limits.get("max_inflight") if isinstance(limits, dict) else None
        if isinstance(inflight, int) and inflight > 0:
            self._slots = threading.BoundedSemaphore(inflight)

    def send(
        self, method: str, params: Mapping[str, Any], execution: Optional[ProcessExecution] = None
    ) -> Tuple[int, _Pending]:
        """Send a request; the returned record completes with its outcome."""
        identifier = next(self._ids)
        slots = None if method in UNLIMITED_METHODS else self._slots
        pending = _Pending(execution, slots is not None)
        if slots is not None:
            slots.acquire()
        with self._lock:
            if self.dead:
                if slots is not None:
                    slots.release()
                raise NurtError("NURT_E_INTERNAL", "nurt serve is not running")
            self._pending[identifier] = pending
        message = {"jsonrpc": "2.0", "id": identifier, "method": method, "params": dict(params)}
        try:
            with self._write_lock:
                self._stdin.write(_encode(message))
                self._stdin.flush()
        except (OSError, ValueError):
            self._complete(identifier, NurtError("NURT_E_INTERNAL", "nurt serve closed its input"))
        return identifier, pending

    def call(self, method: str, params: Mapping[str, Any]) -> Any:
        """Send a request and wait for its result."""
        _, pending = self.send(method, params)
        pending.done.wait()
        return _outcome(pending.outcome)

    def _complete(self, identifier: int, outcome: Outcome) -> None:
        with self._lock:
            pending = self._pending.pop(identifier, None)
        if pending is None:
            return
        if pending.slot and self._slots is not None:
            self._slots.release()
        pending.outcome = outcome
        if pending.execution is not None:
            pending.execution.finish()
        pending.done.set()

    def _read_stdout(self) -> None:
        stream = self.process.stdout
        reason: Optional[str] = None
        try:
            if stream is None:
                return
            for line in iter(stream.readline, b""):
                if not line.strip():
                    continue
                try:
                    message = json.loads(line.decode("utf-8"))
                except ValueError:
                    reason = "nurt serve sent a line that is not JSON"
                    break
                if isinstance(message, dict):
                    self._dispatch(message)
        except (OSError, ValueError):
            reason = "the output of nurt serve cannot be read"
        finally:
            self._exited(reason)

    def _dispatch(self, message: Dict[str, Any]) -> None:
        identifier = message.get("id")
        if isinstance(identifier, int) and ("result" in message or "error" in message):
            self._complete(identifier, message)
            return
        if message.get("method") != EVENT_METHOD:
            return
        params = message.get("params")
        if not isinstance(params, dict):
            return
        with self._lock:
            pending = self._pending.get(params.get("request_id"))  # type: ignore[arg-type]
        if pending is not None and pending.execution is not None:
            pending.execution.deliver(params)

    def _read_stderr(self) -> None:
        stream = self.process.stderr
        if stream is None:
            return
        try:
            for line in iter(stream.readline, b""):
                text = line.decode("utf-8", "replace").rstrip()
                self._stderr.append(text)
                logger.debug("nurt serve: %s", text)
        except (OSError, ValueError):
            return

    def _exited(self, reason: Optional[str]) -> None:
        if reason is not None:
            self.process.kill()
        code = self.process.wait()
        self._stderr_reader.join(SHUTDOWN_TIMEOUT)
        text = reason or f"nurt serve exited with code {code}"
        tail = " | ".join(line for line in self._stderr if line)
        error = NurtError("NURT_E_INTERNAL", f"{text}: {tail}" if tail else text)
        with self._lock:
            self.dead = True
            identifiers = list(self._pending)
        for identifier in identifiers:
            self._complete(identifier, error)

    def terminate(self) -> None:
        """Stop the process: ``shutdown``, end of input, then kill."""
        if not self.dead:
            try:
                _, pending = self.send("shutdown", {})
                pending.done.wait(SHUTDOWN_TIMEOUT)
            except NurtError:
                pass
        try:
            self._stdin.close()
        except OSError:
            pass
        try:
            self.process.wait(SHUTDOWN_TIMEOUT)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()

    def abandon(self) -> None:
        """Drop a connection inherited through ``fork()`` without stopping its process."""
        self.dead = True
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            if stream is not None:
                try:
                    stream.close()
                except OSError:
                    pass


class ProcessExecution(ExecutionHandle):
    """Request ``run`` sent with events enabled."""

    def __init__(self, sink: EventSink) -> None:
        self._sink = sink
        self._events: "queue.Queue[Any]" = queue.Queue()
        self._connection: Optional[_Connection] = None
        self._identifier = 0
        self._pending: Optional[_Pending] = None
        self._dispatcher = threading.Thread(
            target=self._dispatch, name="nurt-serve-events", daemon=True
        )
        self._dispatcher.start()

    def attach(self, connection: _Connection, identifier: int, pending: _Pending) -> None:
        """Record the connection and request carrying the execution."""
        self._connection = connection
        self._identifier = identifier
        self._pending = pending

    def deliver(self, event: Dict[str, Any]) -> None:
        """Queue an event for the sink."""
        self._events.put(event)

    def finish(self) -> None:
        """Mark the end of the events after the response arrived."""
        self._events.put(_END)

    def _dispatch(self) -> None:
        while True:
            event = self._events.get()
            if event is _END:
                self._sink(None)
                return
            self._sink(event)

    def wait(self, timeout: Optional[float]) -> Json:
        pending = self._pending
        if pending is None:
            raise NurtError("NURT_E_ARG", "the execution was not started")
        if not pending.done.wait(timeout):
            raise NurtTimeoutError("NURT_E_TIMEOUT", f"the execution is still running after {timeout} s")
        result = _outcome(pending.outcome)
        if not isinstance(result, dict):
            raise NurtError("NURT_E_INTERNAL", "invalid result of run")
        return result

    def cancel(self) -> None:
        pending, connection = self._pending, self._connection
        if pending is None or connection is None or pending.done.is_set():
            return
        if connection.pid != os.getpid():
            raise NurtError("NURT_E_INTERNAL", "the execution belongs to the parent process")
        try:
            connection.call("cancel", {"request_id": self._identifier})
        except NurtError:
            if not pending.done.is_set():
                raise

    def close(self) -> None:
        try:
            self.cancel()
        except NurtError:
            pass


class ProcessBackend(Backend):
    """Engine of a child process ``nurt serve --stdio``, restarted after it exits."""

    name = "process"

    def __init__(
        self,
        executable: str,
        *,
        config: Optional[Mapping[str, Any]],
        session: Optional[str],
        language: str,
    ) -> None:
        self._argv = [executable, "serve", "--stdio"]
        initialize: Dict[str, Any] = {
            "protocol": PROTOCOL,
            "client": {"name": CLIENT_NAME, "version": __version__},
            "language": language,
            "capabilities": {"events": True},
        }
        if session is not None:
            initialize["session"] = session
        if config is not None:
            initialize["config_overrides"] = dict(config)
        self._initialize = initialize
        self._lock = threading.Lock()
        self._connection: Optional[_Connection] = None
        self._closed = False

    def _connect(self) -> _Connection:
        with self._lock:
            if self._closed:
                raise NurtError("NURT_E_ARG", "the engine is closed")
            connection = self._connection
            if connection is not None and connection.pid != os.getpid():
                connection.abandon()
                connection = None
            if connection is None or connection.dead:
                connection = _Connection(self._argv, self._initialize)
                self._connection = connection
            return connection

    def _call(self, method: str, params: Mapping[str, Any]) -> Any:
        return self._connect().call(method, params)

    def _object(self, method: str, params: Mapping[str, Any]) -> Json:
        result = self._call(method, params)
        if not isinstance(result, dict):
            raise NurtError("NURT_E_INTERNAL", f"invalid result of {method}")
        return result

    def _text(self, method: str, params: Mapping[str, Any]) -> str:
        result = self._object(method, params)
        text = result.get("text")
        if not isinstance(text, str):
            raise NurtError("NURT_E_INTERNAL", f"invalid result of {method}")
        return text

    def run(self, request: Mapping[str, Any]) -> Json:
        return self._object("run", request)

    def start(self, request: Mapping[str, Any], sink: EventSink) -> ExecutionHandle:
        connection = self._connect()
        execution = ProcessExecution(sink)
        try:
            identifier, pending = connection.send("run", request, execution)
        except NurtError:
            execution.finish()
            raise
        execution.attach(connection, identifier, pending)
        return execution

    def check(self, request: Mapping[str, Any]) -> Tuple[Json, Diagnostics]:
        result = self._object("check", request)
        diagnostics = result.pop("diagnostics", [])
        return result, _diagnostics(diagnostics)

    def parse(self, program: str, variant: str) -> Tuple[Optional[Json], Diagnostics]:
        result = self._object("parse", {"program": program, "variant": variant})
        ast = result.get("ast")
        return (ast if isinstance(ast, dict) else None), _diagnostics(result.get("diagnostics"))

    def print(self, ast: Mapping[str, Any], variant: str) -> str:
        return self._text("print", {"ast": dict(ast), "variant": variant})

    def correct(self, program: str, variant: str) -> Json:
        return self._object("correct", {"program": program, "variant": variant})

    def page(self, cursor: str, limit: Optional[int]) -> Json:
        params: Dict[str, Any] = {"cursor": cursor}
        if limit:
            params["limit"] = limit
        return self._object("page", params)

    def describe(self, what: Mapping[str, Any]) -> str:
        return self._text("describe", what)

    def help(self, topic: Optional[str]) -> str:
        return self._text("help", {} if topic is None else {"topic": topic})

    def catalog(self, what: Optional[Mapping[str, Any]]) -> Json:
        return self._object("catalog", {} if what is None else {"what": dict(what)})

    def schema(self, which: str) -> Json:
        return self._object("schema", {"which": which})

    def history(self, query: Optional[Mapping[str, Any]]) -> Json:
        return self._object("history", {} if query is None else {"query": dict(query)})

    def space(self, volume: Optional[str]) -> Json:
        return self._object("space", {} if volume is None else {"volume": volume})

    def info(self) -> Json:
        return self._object("info", {})

    def close(self) -> None:
        with self._lock:
            if self._closed:
                return
            self._closed = True
            connection, self._connection = self._connection, None
        if connection is None:
            return
        if connection.pid != os.getpid():
            connection.abandon()
        else:
            connection.terminate()


def _diagnostics(value: Any) -> Diagnostics:
    items: List[Any] = value if isinstance(value, list) else []
    return [item for item in items if isinstance(item, dict)]
