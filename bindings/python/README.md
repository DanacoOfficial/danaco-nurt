<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# nurt

Danaco Nurt is a command shell and language where every change can be undone. This package is the Python binding.

## Installation

```sh
pip install nurt
```

Platform wheels contain the `nurt` executable and the `libnurt` library for
Linux (glibc and musl), macOS and Windows on x86-64 and ARM64. The source
distribution contains no binaries; it uses the files named by `NURT_LIBRARY`
and `NURT_EXECUTABLE`, or `nurt` found on `PATH`.

The package installs the console script `nurt`, which runs the bundled
executable:

```sh
pipx run nurt --version
```

## Usage

```python
from nurt import Nurt

with Nurt() as nurt:
    response = nurt.run('find "/home/user/project" glob: "**/*.py"', limit=20)
    if response.ok:
        print(response.text)
    else:
        print(response.error["code"], response.error.get("fix"))
```

`Nurt(backend=...)` selects how the engine runs:

| Backend | Engine |
|---|---|
| `"library"` | `libnurt` in the calling process, through `ctypes` |
| `"process"` | a child process `nurt serve --stdio` |
| `"auto"` (default) | `library` when the library is present and compatible, otherwise `process` |

Language errors are returned in the response (`response.ok` is `False`);
`raise_on_error=True` raises `NurtLanguageError` instead. Failures of the call
itself raise `NurtError`, whose `status` is the name of the `nurt_status` code.
Both backends report the language error of `check`, `parse` and `correct` the
same way: the result is the response `nurt-response/1` of the server, with the
status `failed` and the error in `report.error`, and `ParseResult.error` and
`CheckResult.error` read it from there. `catalog`, `history` and `space` raise
`NurtLanguageError` for such an error.
`Nurt.start` returns an `Execution` with events, timeout and cancellation; a
cancelled execution returns its response with the error `E_CANCELLED`.
`AsyncNurt` provides the same methods as coroutines.

## Documentation

https://nurt.danaco-group.pl

## License

Apache-2.0. See `LICENSE` and `NOTICE`.
