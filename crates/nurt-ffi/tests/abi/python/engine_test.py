# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           crates/nurt-ffi/tests/abi/python/engine_test.py
# Responsibility:   Script engine_test.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Interface test of libnurt through ctypes, against the engine of nurt-core
in a build without the cargo feature `stub`.

Usage: python3 -I engine_test.py <path of the shared library>

The engine of this crate has no language front end, so programs end with
language errors in the JSON results. Every status returned by the engine is
checked against the status table of nurt.h. The state directory of the
engine is placed under $HOME. The script prints one line per failure and
exits with status 1 when a check fails.
"""

import ctypes
import json
import os
import sys

OK, TIMEOUT, CANCELLED, ARG, ABI, IO, INTERNAL = range(7)
ABI_VERSION = 1 << 16
WAIT_FOREVER = 0xFFFFFFFF
VARIANT_A = 1

READ = b'{"nurt":"1.0","program":"find \\"/srv/app\\""}'
TWO_INPUTS = b'{"nurt":"1.0","program":"find \\"/srv/app\\"","cursor":"c_91fe"}'
PROGRAM = b'find "/srv/app"'
AST = b'{"nurt":"1.0","program":{"statements":[]}}'
EXTENSION = b"/opt/nurt/ext/sample/nurt-ext.toml"

USUAL = [OK, ARG, INTERNAL]
MATRIX = {
    "nurt_engine_config_get": USUAL,
    "nurt_parse": USUAL,
    "nurt_print": USUAL,
    "nurt_correct": USUAL,
    "nurt_check": USUAL,
    "nurt_run_json": USUAL,
    "nurt_exec_start": USUAL,
    "nurt_page": USUAL,
    "nurt_catalog": USUAL,
    "nurt_describe": USUAL,
    "nurt_help": USUAL,
    "nurt_history": USUAL,
    "nurt_space": USUAL,
    "nurt_session_end": USUAL + [IO],
    "nurt_ext_load": USUAL + [ABI, IO],
    "nurt_ext_list": USUAL,
}


class Buf(ctypes.Structure):
    _fields_ = [("ptr", ctypes.c_void_p), ("len", ctypes.c_size_t), ("cap", ctypes.c_size_t)]


EVENT_CB = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_char_p)
P = ctypes.c_void_p
S = ctypes.c_char_p
B = ctypes.POINTER(Buf)
I = ctypes.c_int
U = ctypes.c_uint32

SIGNATURES = {
    "nurt_abi_version": (U, []),
    "nurt_abi_check": (I, [U]),
    "nurt_version": (S, []),
    "nurt_info": (I, [B]),
    "nurt_engine_open": (I, [S, ctypes.POINTER(P)]),
    "nurt_engine_close": (None, [P]),
    "nurt_engine_config_get": (I, [P, B]),
    "nurt_parse": (I, [P, S, I, B, B]),
    "nurt_print": (I, [P, S, I, B]),
    "nurt_correct": (I, [P, S, I, B]),
    "nurt_check": (I, [P, S, B, B]),
    "nurt_run_json": (I, [P, S, B]),
    "nurt_exec_start": (I, [P, S, EVENT_CB, P, ctypes.POINTER(P)]),
    "nurt_exec_wait": (I, [P, U, B]),
    "nurt_exec_cancel": (I, [P]),
    "nurt_exec_free": (None, [P]),
    "nurt_page": (I, [P, S, U, B]),
    "nurt_catalog": (I, [P, S, B]),
    "nurt_describe": (I, [P, S, B]),
    "nurt_help": (I, [P, S, B]),
    "nurt_schema": (I, [S, B]),
    "nurt_history": (I, [P, S, B]),
    "nurt_space": (I, [P, S, B]),
    "nurt_session_end": (I, [P, S]),
    "nurt_ext_load": (I, [P, S]),
    "nurt_ext_list": (I, [P, B]),
    "nurt_buf_free": (None, [B]),
    "nurt_last_error": (I, [P, B]),
}

failures = []


def check(condition, what):
    if not condition:
        failures.append(what)
        print("check failed: " + what, file=sys.stderr)


def load(path):
    lib = ctypes.CDLL(path)
    for name, (restype, argtypes) in SIGNATURES.items():
        function = getattr(lib, name)
        function.restype = restype
        function.argtypes = argtypes
    return lib


def text(lib, buf):
    """Returns the content of a buffer and releases it."""
    content = ctypes.string_at(buf.ptr, buf.len).decode("utf-8") if buf.ptr else ""
    lib.nurt_buf_free(buf)
    return content


def open_engine(lib, config=None):
    e = P()
    status = lib.nurt_engine_open(config, ctypes.byref(e))
    return status, e


def last_error(lib):
    out = Buf()
    lib.nurt_last_error(None, out)
    content = text(lib, out)
    return json.loads(content) if content else None


def engine_call(lib, function, e):
    """Calls a function that takes an engine with valid other arguments."""
    a, b = Buf(), Buf()
    if function == "nurt_engine_config_get":
        status = lib.nurt_engine_config_get(e, a)
    elif function == "nurt_parse":
        status = lib.nurt_parse(e, PROGRAM, VARIANT_A, a, b)
    elif function == "nurt_print":
        status = lib.nurt_print(e, AST, VARIANT_A, a)
    elif function == "nurt_correct":
        status = lib.nurt_correct(e, PROGRAM, VARIANT_A, a)
    elif function == "nurt_check":
        status = lib.nurt_check(e, READ, a, b)
    elif function == "nurt_run_json":
        status = lib.nurt_run_json(e, READ, a)
    elif function == "nurt_exec_start":
        x = P()
        status = lib.nurt_exec_start(e, READ, EVENT_CB(), None, ctypes.byref(x))
        lib.nurt_exec_free(x)
    elif function == "nurt_page":
        status = lib.nurt_page(e, b"c_91fe", 0, a)
    elif function == "nurt_catalog":
        status = lib.nurt_catalog(e, None, a)
    elif function == "nurt_describe":
        status = lib.nurt_describe(e, None, a)
    elif function == "nurt_help":
        status = lib.nurt_help(e, None, a)
    elif function == "nurt_history":
        status = lib.nurt_history(e, None, a)
    elif function == "nurt_space":
        status = lib.nurt_space(e, None, a)
    elif function == "nurt_session_end":
        status = lib.nurt_session_end(e, b"s-1")
    elif function == "nurt_ext_load":
        status = lib.nurt_ext_load(e, EXTENSION)
    elif function == "nurt_ext_list":
        status = lib.nurt_ext_list(e, a)
    else:
        raise ValueError("no engine scenario for " + function)
    lib.nurt_buf_free(a)
    lib.nurt_buf_free(b)
    return status


def test_matrix(lib):
    status, e = open_engine(lib)
    check(status == OK, "open the engine")
    for function, allowed in MATRIX.items():
        status = engine_call(lib, function, e)
        check(status in allowed, "%s returned %d" % (function, status))
        check(engine_call(lib, function, None) == ARG, function + " with NULL")
    out = Buf()
    check(lib.nurt_abi_check(ABI_VERSION) == OK, "nurt_abi_check")
    check(lib.nurt_abi_check(2 << 16) == ABI, "nurt_abi_check ABI")
    check(lib.nurt_info(out) == OK, "nurt_info")
    check(json.loads(text(lib, out))["catalog_rev"].startswith("sha256:"), "catalog revision")
    check(lib.nurt_info(None) == ARG, "nurt_info with NULL")
    check(lib.nurt_engine_open(None, None) == ARG, "nurt_engine_open with NULL")
    check(lib.nurt_schema(b"ast", out) == OK, "nurt_schema")
    text(lib, out)
    check(lib.nurt_schema(b"unknown", out) == ARG, "nurt_schema with an unknown name")
    check(lib.nurt_last_error(None, None) == ARG, "nurt_last_error with NULL")
    x = P()
    check(lib.nurt_exec_start(e, READ, EVENT_CB(), None, ctypes.byref(x)) == OK, "nurt_exec_start")
    check(lib.nurt_exec_cancel(x) == OK, "nurt_exec_cancel")
    check(lib.nurt_exec_wait(x, WAIT_FOREVER, out) in (OK, CANCELLED), "nurt_exec_wait")
    text(lib, out)
    lib.nurt_exec_free(x)
    check(lib.nurt_exec_wait(None, 0, out) == ARG, "nurt_exec_wait with NULL")
    check(lib.nurt_exec_cancel(None) == ARG, "nurt_exec_cancel with NULL")
    lib.nurt_engine_close(e)


def test_results(lib):
    _, e = open_engine(lib)
    out, diag = Buf(), Buf()
    check(lib.nurt_run_json(e, READ, out) == OK, "read request")
    check(json.loads(text(lib, out))["report"]["error"]["code"] == "E_UNSUPPORTED", "read request without a front end")
    check(lib.nurt_run_json(e, TWO_INPUTS, out) == OK, "envelope rule keeps NURT_OK")
    check(json.loads(text(lib, out))["report"]["error"]["code"] == "E_ARG", "language error E_ARG")
    check(lib.nurt_run_json(e, b"not json", out) == ARG, "invalid JSON")
    check(not out.ptr and out.len == 0, "failing call leaves the buffer empty")
    record = last_error(lib)
    check(record["code"] == "NURT_E_ARG" and record["detail"]["function"] == "nurt_run_json", "last error record")
    check(lib.nurt_parse(e, PROGRAM, VARIANT_A, out, diag) == OK, "parse keeps NURT_OK")
    check(text(lib, out) == "null", "no syntax tree")
    check(json.loads(text(lib, diag))["error"]["code"] == "E_UNSUPPORTED", "language error in the diagnostics")
    check(lib.nurt_space(e, b"srv", out) == OK, "relative volume keeps NURT_OK")
    check(json.loads(text(lib, out))["error"]["code"] == "E_PATH_RELATIVE", "relative volume")
    check(lib.nurt_help(e, b"find", out) == ARG, "help without a front end")
    check(last_error(lib)["detail"]["error"]["code"] == "E_UNSUPPORTED", "language error in the last error")
    lib.nurt_engine_close(e)


def test_events(lib):
    _, e = open_engine(lib)
    received = []
    callback = EVENT_CB(lambda user, event: received.append(json.loads(event.decode("utf-8"))))
    x = P()
    out = Buf()
    check(lib.nurt_exec_start(e, READ, callback, None, ctypes.byref(x)) == OK, "start with events")
    check(lib.nurt_exec_wait(x, WAIT_FOREVER, out) == OK, "wait for completion")
    text(lib, out)
    lib.nurt_exec_free(x)
    check([event["t"] for event in received] == ["exec.started", "exec.finished"], "event types")
    check([event["seq"] for event in received] == [1, 2], "seq increases from 1")
    check(len({event["request"] for event in received}) == 1, "one request identifier")
    check(received[-1]["status"] == "failed", "status of the report in exec.finished")
    lib.nurt_engine_close(e)


def test_open(lib):
    blocker = os.path.join(os.environ["HOME"], "blocker")
    with open(blocker, "w", encoding="utf-8"):
        pass
    config = json.dumps({"state": {"dir": os.path.join(blocker, "state")}}).encode("utf-8")
    status, e = open_engine(lib, config)
    check(status == IO and not e.value, "state directory under a file")
    status, _ = open_engine(lib, b'{"state":{"dir":"state"}}')
    check(status == ARG, "relative state directory")
    status, _ = open_engine(lib, b"[1]")
    check(status == ARG, "configuration that is not an object")


def main():
    lib = load(sys.argv[1])
    check(lib.nurt_abi_version() == ABI_VERSION, "interface version")
    check(len(lib.nurt_version()) > 0, "library version")
    test_matrix(lib)
    test_results(lib)
    test_events(lib)
    test_open(lib)
    if failures:
        print("%d checks failed" % len(failures), file=sys.stderr)
        return 1
    print("ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
