# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Location of the ``nurt`` executable and of the ``libnurt`` library.

Both are searched in the same order: the argument of the caller, the
environment variable (``NURT_EXECUTABLE`` or ``NURT_LIBRARY``), the file
bundled with a platform wheel in ``nurt/_bin``, and, for the executable only,
``PATH``. A location given explicitly (argument or variable) that does not
name a file is an error; the search does not continue past it.
"""

from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path
from typing import List, Mapping, Optional, Sequence, Tuple, Union

from ._errors import NurtError

StrPath = Union[str, "os.PathLike[str]"]

BUNDLE_DIR = Path(__file__).resolve().parent / "_bin"
"""Directory of the files bundled with a platform wheel."""

EXECUTABLE_VARIABLE = "NURT_EXECUTABLE"
LIBRARY_VARIABLE = "NURT_LIBRARY"

LAUNCHER_MARKER = b"nurt._cli"
"""Byte string present in the console script installed for this package."""

LAUNCHER_SCAN_LIMIT = 1 << 20
"""Largest file inspected when telling the console script from the program."""


def executable_names(platform: str = sys.platform) -> Tuple[str, ...]:
    """File names of the ``nurt`` executable on ``platform``."""
    return ("nurt.exe",) if platform.startswith("win") else ("nurt",)


def library_names(platform: str = sys.platform) -> Tuple[str, ...]:
    """File names of the shared ``libnurt`` library on ``platform``."""
    if platform.startswith("win"):
        return ("nurt.dll",)
    if platform == "darwin":
        return ("libnurt.1.dylib", "libnurt.dylib")
    return ("libnurt.so.1", "libnurt.so")


def is_console_launcher(path: StrPath) -> bool:
    """Return ``True`` when ``path`` is the console script of this package.

    The console script runs the bundled or located executable itself, so
    selecting it from ``PATH`` would start the search again.
    """
    try:
        with open(path, "rb") as stream:
            return LAUNCHER_MARKER in stream.read(LAUNCHER_SCAN_LIMIT)
    except OSError:
        return False


def find_executable(
    explicit: Optional[StrPath] = None,
    *,
    environ: Optional[Mapping[str, str]] = None,
    bundle_dir: Path = BUNDLE_DIR,
) -> str:
    """Return the path of the ``nurt`` executable.

    Raises :class:`nurt.NurtError` with status ``NURT_E_IO`` and the list of
    checked locations when no executable is found.
    """
    env = os.environ if environ is None else environ
    checked: List[str] = []
    if _given(explicit):
        found = _explicit(explicit, "argument executable", checked)
    elif _given(env.get(EXECUTABLE_VARIABLE)):
        found = _explicit(env.get(EXECUTABLE_VARIABLE), EXECUTABLE_VARIABLE, checked)
    else:
        found = _bundled(bundle_dir, executable_names(), checked)
        if found is None:
            found = _on_path(env.get("PATH", ""), checked)
    if found is None:
        raise _not_found("nurt executable", checked, EXECUTABLE_VARIABLE)
    return found


def find_library(
    explicit: Optional[StrPath] = None,
    *,
    environ: Optional[Mapping[str, str]] = None,
    bundle_dir: Path = BUNDLE_DIR,
) -> str:
    """Return the path of the shared ``libnurt`` library.

    Raises :class:`nurt.NurtError` with status ``NURT_E_IO`` and the list of
    checked locations when no library is found.
    """
    env = os.environ if environ is None else environ
    checked: List[str] = []
    if _given(explicit):
        found = _explicit(explicit, "argument library", checked)
    elif _given(env.get(LIBRARY_VARIABLE)):
        found = _explicit(env.get(LIBRARY_VARIABLE), LIBRARY_VARIABLE, checked)
    else:
        found = _bundled(bundle_dir, library_names(), checked)
    if found is None:
        raise _not_found("libnurt library", checked, LIBRARY_VARIABLE)
    return found


def _given(value: Optional[StrPath]) -> bool:
    return value is not None and os.fspath(value) != ""


def _explicit(value: Optional[StrPath], source: str, checked: List[str]) -> Optional[str]:
    if value is None:
        return None
    path = os.fspath(value)
    if os.path.isfile(path):
        return os.path.abspath(path)
    checked.append(f"{path} ({source})")
    return None


def _bundled(bundle_dir: Path, names: Sequence[str], checked: List[str]) -> Optional[str]:
    for name in names:
        candidate = bundle_dir / name
        if candidate.is_file():
            return str(candidate)
        checked.append(f"{candidate} (bundled)")
    return None


def _on_path(search_path: str, checked: List[str]) -> Optional[str]:
    for directory in search_path.split(os.pathsep):
        if not directory:
            continue
        candidate = shutil.which("nurt", path=directory)
        if candidate is None:
            continue
        if is_console_launcher(candidate):
            checked.append(f"{candidate} (PATH, console script of this package)")
            continue
        return os.path.abspath(candidate)
    checked.append("PATH")
    return None


def _not_found(what: str, checked: List[str], variable: str) -> NurtError:
    places = "; ".join(checked) if checked else "none"
    return NurtError(
        "NURT_E_IO",
        f"{what} not found; checked: {places}; set {variable} or install a platform wheel",
        detail={"checked": list(checked)},
    )
