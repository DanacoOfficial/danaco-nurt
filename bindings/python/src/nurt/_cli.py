# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Console script ``nurt``: runs the located ``nurt`` executable."""

from __future__ import annotations

import os
import signal
import subprocess
import sys
from typing import List, NoReturn, Optional

from ._errors import NurtError
from ._locate import find_executable

EXIT_NOT_FOUND = 127
"""Exit code when the executable cannot be found, as for a missing command in a shell."""


def main(argv: Optional[List[str]] = None) -> NoReturn:
    """Replace the process with ``nurt`` (POSIX) or run it and exit with its code (Windows)."""
    arguments = sys.argv[1:] if argv is None else argv
    try:
        executable = find_executable()
    except NurtError as exc:
        sys.stderr.write(f"nurt: {exc.message}\n")
        sys.exit(EXIT_NOT_FOUND)
    command = [executable, *arguments]
    if os.name != "nt":
        os.execv(executable, command)
    signal.signal(signal.SIGINT, signal.SIG_IGN)
    sys.exit(subprocess.call(command))
