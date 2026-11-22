# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Mock artifacts and shared helpers of the tests.

``NURT_TEST_EXECUTABLE`` and ``NURT_TEST_LIBRARY`` replace the mocks with
other implementations, for example the stub build of ``libnurt`` or the
conformance server. Tests that need the controls of the mock library are
skipped for other libraries.
"""

from __future__ import annotations

import atexit
import ctypes
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Callable, Optional

TESTS_DIR = Path(__file__).resolve().parent
PACKAGE_DIR = TESTS_DIR.parent
REPOSITORY_DIR = PACKAGE_DIR.parent.parent
MOCK_DIR = TESTS_DIR / "mock"
HEADER = REPOSITORY_DIR / "crates" / "nurt-ffi" / "include" / "nurt.h"
SPEC_DIR = REPOSITORY_DIR / "spec"

sys.path.insert(0, str(PACKAGE_DIR / "src"))

_scratch: Optional[Path] = None
_executable: Optional[str] = None
_library: Optional[str] = None
_library_problem: Optional[str] = None


def scratch() -> Path:
    """Directory removed when the test process exits."""
    global _scratch
    if _scratch is None:
        _scratch = Path(tempfile.mkdtemp(prefix="nurt-python-tests-"))
        atexit.register(shutil.rmtree, _scratch, True)
    return _scratch


def write_launcher(directory: Path, name: str = "nurt") -> str:
    """Write an executable that runs the mock server and return its path."""
    directory.mkdir(parents=True, exist_ok=True)
    server = MOCK_DIR / "mock_server.py"
    if os.name == "nt":
        path = directory / f"{name}.cmd"
        path.write_text(f'@"{sys.executable}" "{server}" %*\r\n', encoding="utf-8")
    else:
        path = directory / name
        path.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{server}" "$@"\n', encoding="utf-8")
        path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return str(path)


def mock_executable() -> str:
    """Path of the server used by the process backend tests."""
    global _executable
    if _executable is None:
        _executable = os.environ.get("NURT_TEST_EXECUTABLE") or write_launcher(scratch() / "server")
    return _executable


def library_file_name() -> str:
    """File name of a shared library on this system."""
    if sys.platform == "darwin":
        return "libnurt.1.dylib"
    if os.name == "nt":
        return "nurt.dll"
    return "libnurt.so.1"


def build_mock_library(directory: Path) -> str:
    """Compile ``mock_libnurt.c`` into ``directory`` and return the library path."""
    compiler = os.environ.get("CC") or shutil.which("cc") or shutil.which("clang") or shutil.which("gcc")
    if compiler is None:
        raise unittest.SkipTest("no C compiler for the mock library")
    if os.name == "nt":
        raise unittest.SkipTest("the mock library needs POSIX threads")
    if not HEADER.is_file():
        raise unittest.SkipTest("nurt.h is not available outside the repository")
    directory.mkdir(parents=True, exist_ok=True)
    output = directory / library_file_name()
    link = ["-dynamiclib"] if sys.platform == "darwin" else ["-shared"]
    command = [
        compiler,
        *link,
        "-fPIC",
        "-O1",
        "-std=c11",
        "-pthread",
        "-Wall",
        "-Werror",
        f"-I{HEADER.parent}",
        "-o",
        str(output),
        str(MOCK_DIR / "mock_libnurt.c"),
    ]
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode != 0:
        raise AssertionError(f"mock library does not compile:\n{result.stderr}")
    return str(output)


def mock_library() -> str:
    """Path of the library used by the library backend tests; skips when unavailable."""
    global _library, _library_problem
    if _library is None and _library_problem is None:
        explicit = os.environ.get("NURT_TEST_LIBRARY")
        if explicit:
            _library = explicit
        else:
            try:
                _library = build_mock_library(scratch() / "library")
            except unittest.SkipTest as skip:
                _library_problem = str(skip)
    if _library is None:
        raise unittest.SkipTest(_library_problem or "no library")
    return _library


class MockControls:
    """Counters and failure injection of ``mock_libnurt.c``."""

    def __init__(self, dll: ctypes.CDLL) -> None:
        self._dll = dll
        for name in ("nurt_mock_live_buffers", "nurt_mock_live_executions", "nurt_mock_live_engines"):
            getattr(dll, name).restype = ctypes.c_long
            getattr(dll, name).argtypes = []
        dll.nurt_mock_fail_next.restype = None
        dll.nurt_mock_fail_next.argtypes = [ctypes.c_char_p, ctypes.c_int]

    def live_buffers(self) -> int:
        return int(self._dll.nurt_mock_live_buffers())

    def live_executions(self) -> int:
        return int(self._dll.nurt_mock_live_executions())

    def live_engines(self) -> int:
        return int(self._dll.nurt_mock_live_engines())

    def fail_next(self, function: str, status: int) -> None:
        self._dll.nurt_mock_fail_next(function.encode("ascii"), status)


def mock_controls(path: str) -> MockControls:
    """Controls of the loaded mock library; skips for other libraries."""
    from nurt._library import load

    dll = load(path).dll
    if not hasattr(dll, "nurt_mock_live_buffers"):
        raise unittest.SkipTest("the library under test is not the mock")
    return MockControls(dll)


def requires_fork(test: Callable[..., None]) -> Callable[..., None]:
    """Skip ``test`` on systems without ``os.fork``."""
    return unittest.skipUnless(hasattr(os, "fork"), "os.fork is not available")(test)
