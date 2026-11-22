# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Python binding of Danaco Nurt.

The binding runs programs of the Danaco Nurt language through ``libnurt`` in
the calling process or through a child process ``nurt serve --stdio``::

    from nurt import Nurt

    with Nurt() as nurt:
        response = nurt.run('find "/home/user/project" glob: "**/*.py"')
        if response.ok:
            print(response.text)
        else:
            print(response.error["code"], response.error.get("fix"))
"""

from ._async import AsyncExecution, AsyncNurt
from ._client import Execution, Nurt
from ._errors import NurtCancelledError, NurtError, NurtLanguageError, NurtTimeoutError
from ._response import CheckResult, CorrectResult, ParseResult, Response
from ._version import __version__

__all__ = [
    "AsyncExecution",
    "AsyncNurt",
    "CheckResult",
    "CorrectResult",
    "Execution",
    "Nurt",
    "NurtCancelledError",
    "NurtError",
    "NurtLanguageError",
    "NurtTimeoutError",
    "ParseResult",
    "Response",
    "__version__",
]
