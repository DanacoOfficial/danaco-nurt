# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Mock of ``nurt serve --stdio`` for the tests of the Python binding.

Speaks ``nurt-rpc/1`` over NDJSON with the same program behaviour as
``mock_libnurt.c``; in addition ``crash`` makes the process exit with code 3
and ``session`` returns the ``session`` and ``language`` of ``initialize``.
``NURT_MOCK_PROTOCOL`` overrides the protocol reported by ``initialize``.
Invoked as ``mock_server.py serve --stdio`` or ``mock_server.py --version``.
"""

from __future__ import annotations

import json
import os
import sys
import threading
import time
from typing import Any, Dict, Optional

VERSION = "0.1.0"
SLICE = 0.005
CRASH_EXIT_CODE = 3
USAGE_EXIT_CODE = 2
SCHEMAS = ("ast", "bound", "report", "request", "config", "catalog", "value", "tool")
STATUS_NAMES = (
    "NURT_OK",
    "NURT_E_TIMEOUT",
    "NURT_E_CANCELLED",
    "NURT_E_ARG",
    "NURT_E_ABI",
    "NURT_E_IO",
    "NURT_E_INTERNAL",
)
SYNTAX_ERROR = {
    "code": "E_SYNTAX",
    "message": "unexpected end of program",
    "stage": "parse",
    "at": {"line": 1, "column": 5},
    "fix": ["complete the program"],
}


class CallFailure(Exception):
    """A call failure with a ``nurt_status`` code."""

    def __init__(self, status: int, message: str, response: Optional[Dict[str, Any]] = None) -> None:
        super().__init__(message)
        self.status = status
        self.response = response


def response(status: str, kind: str, text: Optional[str], **fields: Any) -> Dict[str, Any]:
    report: Dict[str, Any] = {"nurt": "1.0", "status": status, "kind": kind, **fields}
    result: Dict[str, Any] = {"nurt": "1.0", "report": report}
    if text is not None:
        result["rendered"] = {"format": "text", "text": text}
    return result


class Server:
    def __init__(self) -> None:
        self.write_lock = threading.Lock()
        self.state_lock = threading.Lock()
        self.cancelled: Dict[Any, bool] = {}
        self.events = False
        self.defaults: Dict[str, Any] = {}

    def send(self, message: Dict[str, Any]) -> None:
        line = json.dumps(message, separators=(",", ":")) + "\n"
        with self.write_lock:
            sys.stdout.write(line)
            sys.stdout.flush()

    def reply(self, identifier: Any, result: Any) -> None:
        self.send({"jsonrpc": "2.0", "id": identifier, "result": result})

    def error(self, identifier: Any, code: int, message: str, data: Any = None) -> None:
        error: Dict[str, Any] = {"code": code, "message": message}
        if data is not None:
            error["data"] = data
        self.send({"jsonrpc": "2.0", "id": identifier, "error": error})

    def event(self, identifier: Any, kind: str, seq: int) -> None:
        if self.events:
            params = {"t": kind, "seq": seq, "time": "2026-01-01T00:00:00Z", "request": str(identifier), "progress": seq}
            self.send({"jsonrpc": "2.0", "method": "nurt/event", "params": params})

    def serve(self) -> int:
        for line in sys.stdin:
            if not line.strip():
                continue
            message = json.loads(line)
            method = message.get("method")
            if method == "shutdown":
                self.reply(message.get("id"), {})
                return 0
            if method == "run":
                with self.state_lock:
                    self.cancelled[message.get("id")] = False
                threading.Thread(target=self.handle, args=(message,), daemon=True).start()
            else:
                self.handle(message)
        return 0

    def handle(self, message: Dict[str, Any]) -> None:
        identifier = message.get("id")
        params = message.get("params") or {}
        method = message.get("method")
        handler = getattr(self, f"on_{method}", None)
        if handler is None:
            self.error(identifier, -32601, f"unknown method {method}")
            return
        try:
            self.reply(identifier, handler(identifier, params))
        except CallFailure as failure:
            data: Dict[str, Any] = {"status": STATUS_NAMES[failure.status] if failure.status < 7 else "NURT_E_UNKNOWN"}
            if failure.response is not None:
                data["response"] = failure.response
            self.error(identifier, -32000 - failure.status, str(failure), data)

    def on_initialize(self, _identifier: Any, params: Dict[str, Any]) -> Dict[str, Any]:
        self.events = bool(params.get("capabilities", {}).get("events"))
        self.defaults = {key: params[key] for key in ("session", "language") if key in params}
        return {
            "protocol": os.environ.get("NURT_MOCK_PROTOCOL", "nurt-rpc/1"),
            "version": VERSION,
            "abi": "1.0",
            "language": params.get("language", "en"),
            "features": [],
            "limits": {"max_inflight": 64, "max_message_bytes": 16 << 20},
        }

    def on_run(self, identifier: Any, params: Dict[str, Any]) -> Dict[str, Any]:
        program = str(params.get("program", ""))
        seq = 0
        try:
            if program.startswith("events "):
                for _ in range(int(program.split()[1])):
                    seq += 1
                    self.event(identifier, "job.progress", seq)
            if program.startswith("sleep "):
                deadline = time.monotonic() + int(program.split()[1]) / 1000
                while time.monotonic() < deadline and not self.cancelled.get(identifier):
                    time.sleep(SLICE)
            if self.cancelled.get(identifier):
                cancelled = response("failed", "mock", None, error={"code": "E_CANCELLED", "message": "cancelled"})
                raise CallFailure(2, "the execution was cancelled", cancelled)
            if program == "crash":
                sys.stderr.write("mock: crash requested\n")
                sys.stderr.flush()
                os._exit(CRASH_EXIT_CODE)
            return self.respond(program, params)
        finally:
            self.event(identifier, "exec.finished", seq + 1)
            with self.state_lock:
                self.cancelled.pop(identifier, None)

    def respond(self, program: str, request: Dict[str, Any]) -> Dict[str, Any]:
        if program.startswith("status "):
            raise CallFailure(int(program.split()[1]), "failure requested by the program")
        if program == "panic":
            raise CallFailure(6, "panic caught at the boundary")
        if program == "fail":
            return response("failed", "parse", "error E_SYNTAX: unexpected end of program", error=SYNTAX_ERROR)
        if program == "page":
            return response("ok", "mock", "page 1", next_cursor="page-2")
        if program == "options":
            return response("ok", "mock", json.dumps(request.get("options", {}), separators=(",", ":")))
        if program == "session":
            return response("ok", "mock", json.dumps(self.defaults, separators=(",", ":")))
        return response("ok", "mock", program)

    def on_cancel(self, _identifier: Any, params: Dict[str, Any]) -> Dict[str, Any]:
        with self.state_lock:
            if params.get("request_id") in self.cancelled:
                self.cancelled[params["request_id"]] = True
        return {}

    def on_check(self, _identifier: Any, params: Dict[str, Any]) -> Dict[str, Any]:
        program = params.get("program")
        if program is None:
            raise CallFailure(3, "request has no program")
        if program == "fail":
            return response("failed", "check", None, error=SYNTAX_ERROR)
        return {"bound": {"nurt": "1.0", "program": program}, "effects": [], "diagnostics": []}

    def on_parse(self, _identifier: Any, params: Dict[str, Any]) -> Dict[str, Any]:
        program = params.get("program")
        if program == "fail":
            return response("failed", "parse", None, error=SYNTAX_ERROR)
        return {"ast": {"nurt": "1.0", "program": program}, "diagnostics": []}

    def on_print(self, _identifier: Any, params: Dict[str, Any]) -> Dict[str, Any]:
        return {"text": params.get("ast", {}).get("program", "")}

    def on_correct(self, _identifier: Any, params: Dict[str, Any]) -> Dict[str, Any]:
        return {"program": params.get("program"), "corrections": []}

    def on_page(self, _identifier: Any, params: Dict[str, Any]) -> Dict[str, Any]:
        if params.get("cursor") == "page-2":
            return response("ok", "mock", "page 2")
        return response("failed", "page", None, error={"code": "E_ARG", "message": "the cursor has expired"})

    def on_describe(self, _identifier: Any, params: Dict[str, Any]) -> Dict[str, Any]:
        return {"description": "description " + json.dumps(params, separators=(",", ":"))}

    def on_help(self, _identifier: Any, params: Dict[str, Any]) -> Dict[str, Any]:
        return {"text": "help " + str(params.get("topic", "index"))}

    def on_catalog(self, _identifier: Any, _params: Dict[str, Any]) -> Dict[str, Any]:
        return {"catalog_rev": "mock", "groups": []}

    def on_schema(self, _identifier: Any, params: Dict[str, Any]) -> Dict[str, Any]:
        which = params.get("name")
        if which not in SCHEMAS:
            raise CallFailure(3, "unknown schema name")
        return {"title": f"nurt-{which}/1"}

    def on_history(self, _identifier: Any, _params: Dict[str, Any]) -> Dict[str, Any]:
        return {"entries": []}

    def on_space(self, _identifier: Any, _params: Dict[str, Any]) -> Dict[str, Any]:
        return {"volumes": []}

    def on_info(self, _identifier: Any, _params: Dict[str, Any]) -> Dict[str, Any]:
        return {"version": VERSION, "abi": "1.0", "implementation": "mock"}


def main(argv: list) -> int:
    if argv == ["--version"]:
        sys.stdout.write(f"nurt {VERSION}\n")
        return 0
    if argv != ["serve", "--stdio"]:
        sys.stderr.write("usage: nurt serve --stdio\n")
        return USAGE_EXIT_CODE
    return Server().serve()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
