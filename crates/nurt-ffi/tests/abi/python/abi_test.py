# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Interface test of libnurt through ctypes, against the engine of the cargo
feature `stub`.

Usage: python3 -I abi_test.py <path of the shared library>

Every cell of the status table of nurt.h is exercised. The script prints
one line per failure and exits with status 1 when a check fails.
"""

import ctypes
import json
import os
import sys

OK, TIMEOUT, CANCELLED, ARG, ABI, IO, INTERNAL = range(7)
ABI_VERSION = 1 << 16
WAIT_FOREVER = 0xFFFFFFFF
VARIANT_A = 1

READ = b'{"nurt":"1.0","program":"find \\"/srv/app\\" glob: \\"*.go\\""}'
SYNTAX_ERROR = b'{"nurt":"1.0","program":"find \\"/srv/app\\" glob: [\\"*.go\\""}'
TWO_INPUTS = b'{"nurt":"1.0","program":"find \\"/srv/app\\"","cursor":"c_91fe"}'
EVENTS = b'{"nurt":"1.0","program":"find \\"/srv/app\\" glob: \\"*.rs\\""}'
SLOW = b'{"nurt":"1.0","program":"find \\"/srv/archive\\""}'
PROGRAM = b'find "/srv/app"'
MISSPELT = b'fnd "/srv/app"'
AST = b'{"nurt":"1.0","program":{"statements":[{"t":"pipeline","head":{"t":"stage","name":"find","args":[{"value":{"t":"text","v":"/srv/app"}}]},"stages":[]}]}}'

MATRIX = {
    "nurt_abi_check": [OK, ABI],
    "nurt_info": [OK, ARG, INTERNAL],
    "nurt_engine_open": [OK, ARG, IO, INTERNAL],
    "nurt_engine_config_get": [OK, ARG, INTERNAL],
    "nurt_parse": [OK, ARG, INTERNAL],
    "nurt_print": [OK, ARG, INTERNAL],
    "nurt_correct": [OK, ARG, INTERNAL],
    "nurt_check": [OK, ARG, INTERNAL],
    "nurt_run_json": [OK, ARG, INTERNAL],
    "nurt_exec_start": [OK, ARG, INTERNAL],
    "nurt_exec_wait": [OK, TIMEOUT, CANCELLED, ARG, INTERNAL],
    "nurt_exec_cancel": [OK, ARG, INTERNAL],
    "nurt_page": [OK, ARG, INTERNAL],
    "nurt_catalog": [OK, ARG, INTERNAL],
    "nurt_describe": [OK, ARG, INTERNAL],
    "nurt_help": [OK, ARG, INTERNAL],
    "nurt_schema": [OK, ARG, INTERNAL],
    "nurt_history": [OK, ARG, INTERNAL],
    "nurt_space": [OK, ARG, INTERNAL],
    "nurt_session_end": [OK, ARG, IO, INTERNAL],
    "nurt_ext_load": [OK, ARG, ABI, IO, INTERNAL],
    "nurt_ext_list": [OK, ARG, INTERNAL],
    "nurt_last_error": [OK, ARG],
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
    value = ctypes.string_at(buf.ptr, buf.len).decode("utf-8") if buf.ptr else ""
    lib.nurt_buf_free(ctypes.byref(buf))
    return value


def open_engine(lib):
    engine = P()
    if lib.nurt_engine_open(None, ctypes.byref(engine)) != OK:
        raise SystemExit("cannot open the stub engine")
    return engine


def engine_call(lib, function, e):
    a, b = Buf(), Buf()
    if function == "nurt_engine_config_get":
        status = lib.nurt_engine_config_get(e, a)
    elif function == "nurt_parse":
        status = lib.nurt_parse(e, PROGRAM, VARIANT_A, a, b)
    elif function == "nurt_print":
        status = lib.nurt_print(e, AST, VARIANT_A, a)
    elif function == "nurt_correct":
        status = lib.nurt_correct(e, MISSPELT, VARIANT_A, a)
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
    elif function in ("nurt_catalog", "nurt_describe", "nurt_help", "nurt_history", "nurt_space"):
        status = getattr(lib, function)(e, None, a)
    elif function == "nurt_session_end":
        status = lib.nurt_session_end(e, b"s-1")
    elif function == "nurt_ext_load":
        status = lib.nurt_ext_load(e, b"/opt/nurt/ext/sample/nurt-ext.toml")
    elif function == "nurt_ext_list":
        status = lib.nurt_ext_list(e, a)
    else:
        raise SystemExit("no engine scenario for " + function)
    text(lib, a)
    text(lib, b)
    return status


def with_exec(lib, e, request, timeout, cancel):
    x = P()
    out = Buf()
    if lib.nurt_exec_start(e, request, EVENT_CB(), None, ctypes.byref(x)) != OK:
        return INTERNAL
    if cancel:
        lib.nurt_exec_cancel(x)
    status = lib.nurt_exec_wait(x, timeout, out)
    text(lib, out)
    lib.nurt_exec_free(x)
    return status


def ok_scenario(lib, function):
    out = Buf()
    if function == "nurt_abi_check":
        return lib.nurt_abi_check(ABI_VERSION)
    if function == "nurt_info":
        status = lib.nurt_info(out)
    elif function == "nurt_engine_open":
        other = P()
        status = lib.nurt_engine_open(None, ctypes.byref(other))
        lib.nurt_engine_close(other)
    elif function == "nurt_schema":
        status = lib.nurt_schema(b"ast", out)
    elif function == "nurt_last_error":
        status = lib.nurt_last_error(None, out)
    else:
        e = open_engine(lib)
        if function == "nurt_exec_wait":
            status = with_exec(lib, e, READ, WAIT_FOREVER, False)
        elif function == "nurt_exec_cancel":
            x = P()
            lib.nurt_exec_start(e, READ, EVENT_CB(), None, ctypes.byref(x))
            status = lib.nurt_exec_cancel(x)
            lib.nurt_exec_free(x)
        else:
            status = engine_call(lib, function, e)
        lib.nurt_engine_close(e)
    text(lib, out)
    return status


def arg_scenario(lib, function):
    out = Buf()
    if function == "nurt_info":
        return lib.nurt_info(None)
    if function == "nurt_engine_open":
        return lib.nurt_engine_open(None, None)
    if function == "nurt_exec_wait":
        return lib.nurt_exec_wait(None, 0, out)
    if function == "nurt_exec_cancel":
        return lib.nurt_exec_cancel(None)
    if function == "nurt_schema":
        status = lib.nurt_schema(b"unknown", out)
        text(lib, out)
        return status
    if function == "nurt_last_error":
        return lib.nurt_last_error(None, None)
    return engine_call(lib, function, None)


def other_scenario(lib, function, expected):
    e = open_engine(lib)
    try:
        if function == "nurt_abi_check" and expected == ABI:
            return lib.nurt_abi_check((1 + 1) << 16)
        if function == "nurt_ext_load" and expected == ABI:
            return lib.nurt_ext_load(e, b"/opt/nurt/ext/future/nurt-ext.toml")
        if function == "nurt_ext_load" and expected == IO:
            return lib.nurt_ext_load(e, b"/opt/nurt/ext/missing/nurt-ext.toml")
        if function == "nurt_session_end" and expected == IO:
            return lib.nurt_session_end(e, b"s-io")
        if function == "nurt_engine_open" and expected == IO:
            other = P()
            os.environ["NURT_STUB_DIR"] = "/nonexistent/nurt-stub-cases"
            try:
                return lib.nurt_engine_open(None, ctypes.byref(other))
            finally:
                del os.environ["NURT_STUB_DIR"]
        if function == "nurt_exec_wait" and expected == TIMEOUT:
            return with_exec(lib, e, SLOW, 0, False)
        if function == "nurt_exec_wait" and expected == CANCELLED:
            return with_exec(lib, e, SLOW, WAIT_FOREVER, True)
        raise SystemExit("no scenario for %s with status %d" % (function, expected))
    finally:
        lib.nurt_engine_close(e)


def scenario(lib, function, expected):
    if expected == OK:
        return ok_scenario(lib, function)
    if expected == ARG:
        return arg_scenario(lib, function)
    if expected == INTERNAL:
        os.environ["NURT_STUB_PANIC"] = function
        try:
            return ok_scenario(lib, function)
        finally:
            del os.environ["NURT_STUB_PANIC"]
    return other_scenario(lib, function, expected)


def test_matrix(lib):
    for function, statuses in MATRIX.items():
        for expected in statuses:
            actual = scenario(lib, function, expected)
            check(actual == expected, "%s: expected status %d, got %d" % (function, expected, actual))


def test_run_json(lib):
    e = open_engine(lib)
    out = Buf()
    check(lib.nurt_run_json(e, READ, out) == OK, "read request")
    check(json.loads(text(lib, out))["report"]["records"]["matched"] == 148, "read response holds the records")
    for request, code in ((SYNTAX_ERROR, "E_SYNTAX"), (TWO_INPUTS, "E_ARG")):
        check(lib.nurt_run_json(e, request, out) == OK, "language error keeps NURT_OK")
        check(json.loads(text(lib, out))["report"]["error"]["code"] == code, "language error " + code)
    check(lib.nurt_run_json(e, b"not json", out) == ARG, "invalid JSON")
    check(not out.ptr and out.len == 0, "failing call leaves the buffer empty")
    check(lib.nurt_last_error(None, out) == OK, "last error")
    record = json.loads(text(lib, out))
    check(record["code"] == "NURT_E_ARG" and record["detail"]["function"] == "nurt_run_json", "last error record")
    lib.nurt_buf_free(out)
    lib.nurt_buf_free(None)
    lib.nurt_engine_close(e)


def test_events(lib):
    e = open_engine(lib)
    received = []
    callback = EVENT_CB(lambda user, event: received.append(json.loads(event.decode("utf-8"))))
    x = P()
    out = Buf()
    check(lib.nurt_exec_start(e, EVENTS, callback, None, ctypes.byref(x)) == OK, "start with events")
    check(lib.nurt_exec_wait(x, WAIT_FOREVER, out) == OK, "wait for completion")
    text(lib, out)
    lib.nurt_exec_free(x)
    check([event["t"] for event in received] == ["exec.started", "log", "progress", "progress", "exec.finished"], "event types")
    check([event["seq"] for event in received] == [1, 2, 3, 4, 5], "seq increases from 1")
    lib.nurt_engine_close(e)


def main():
    lib = load(sys.argv[1])
    check(lib.nurt_abi_version() == ABI_VERSION, "interface version")
    check(len(lib.nurt_version()) > 0, "library version")
    test_matrix(lib)
    test_run_json(lib)
    test_events(lib)
    if failures:
        print("%d checks failed" % len(failures), file=sys.stderr)
        return 1
    print("ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
