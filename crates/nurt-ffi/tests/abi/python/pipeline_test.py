# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Pipeline test of libnurt through ctypes: programs of the form
`find |> filter |> count` and `find |> remove`, with and without `preview`,
are run through nurt_run_json on an engine over a virtual file system (cargo
feature `e2e`, environment variable NURT_E2E_ENGINE). The engine holds the
files /data/a.log (10 bytes), /data/b.log (200) and /data/c.log (300).

Usage: python3 -I pipeline_test.py <path of the shared library>
"""

import ctypes
import json
import sys

OK = 0
LIST = "find /data"
COUNT_LARGE = "find /data |> filter size > 150 |> count"
REMOVE_ALL = "find /data |> remove"
REMOVE_LARGE = "find /data |> filter size > 150 |> remove"


class Buf(ctypes.Structure):
    _fields_ = [("ptr", ctypes.c_void_p), ("len", ctypes.c_size_t), ("cap", ctypes.c_size_t)]


failures = []


def check(condition, what, response=None):
    if not condition:
        failures.append(what)
        print("check failed: %s\n%s" % (what, json.dumps(response)), file=sys.stderr)


def load(path):
    lib = ctypes.CDLL(path)
    lib.nurt_engine_open.restype = ctypes.c_int
    lib.nurt_engine_open.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
    lib.nurt_engine_close.restype = None
    lib.nurt_engine_close.argtypes = [ctypes.c_void_p]
    lib.nurt_run_json.restype = ctypes.c_int
    lib.nurt_run_json.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.POINTER(Buf)]
    lib.nurt_buf_free.restype = None
    lib.nurt_buf_free.argtypes = [ctypes.POINTER(Buf)]
    return lib


class Engine:
    def __init__(self, lib):
        self.lib = lib
        self.handle = ctypes.c_void_p()
        if lib.nurt_engine_open(None, ctypes.byref(self.handle)) != OK:
            raise SystemExit("cannot open the engine")

    def run(self, program, preview=False):
        request = {
            "nurt": "1.0",
            "program": program,
            "options": {"preview": preview, "verbosity": "detailed"},
        }
        out = Buf()
        status = self.lib.nurt_run_json(self.handle, json.dumps(request).encode("utf-8"), ctypes.byref(out))
        if status != OK:
            raise SystemExit("nurt_run_json failed for " + program)
        content = ctypes.string_at(out.ptr, out.len).decode("utf-8")
        self.lib.nurt_buf_free(out)
        return json.loads(content)["report"]

    def paths(self):
        rows = self.run(LIST).get("records", {}).get("rows", [])
        return sorted(row["fields"]["path"]["v"] for row in rows)

    def close(self):
        self.lib.nurt_engine_close(self.handle)


def counted(report):
    rows = report["records"]["rows"]
    return [row["fields"]["value"]["v"] for row in rows]


def deletions(report):
    return [change for change in report.get("changes", []) if change["action"] == "delete"]


def test_count(lib):
    e = Engine(lib)
    report = e.run(COUNT_LARGE)
    check(report["status"] == "ok", "count status", report)
    check(counted(report) == [2], "two files are larger than 150 bytes", report)
    check(not report.get("changes"), "counting changes nothing", report)
    e.close()


def test_remove_with_preview(lib):
    e = Engine(lib)
    report = e.run(REMOVE_ALL, preview=True)
    check(report["status"] == "planned", "preview status", report)
    check(len(deletions(report)) == 3, "preview plans three deletions", report)
    check("op" not in report, "preview has no operation", report)
    check(len(e.paths()) == 3, "preview keeps the files")
    e.close()


def test_remove_without_preview(lib):
    e = Engine(lib)
    report = e.run(REMOVE_LARGE)
    check(report["status"] == "ok", "remove status", report)
    check(len(deletions(report)) == 2, "two deletions", report)
    check("op" in report, "operation number", report)
    check(e.paths() == ["/data/a.log"], "only the small file remains", e.paths())
    check(len(deletions(e.run(REMOVE_ALL))) == 1, "one file left to delete")
    check(counted(e.run(COUNT_LARGE)) == [0], "nothing is left to count")
    e.close()


def main():
    lib = load(sys.argv[1])
    test_count(lib)
    test_remove_with_preview(lib)
    test_remove_without_preview(lib)
    if failures:
        print("%d checks failed" % len(failures), file=sys.stderr)
        return 1
    print("ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
