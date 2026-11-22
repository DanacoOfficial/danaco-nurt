<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Exit codes

The exit code of `nurt` follows the status of the report of the program and the interface status of its error, so that the command line and the C interface describe the same outcomes.

| Code | Name | Meaning | Interface status |
|---|---|---|---|
| 0 | success | The report status is `ok`, `planned` or `running`. | `NURT_OK` |
| 1 | failure | The program failed with a language or execution error, or a file could not be read or written. | `NURT_OK`, `NURT_E_ARG`, `NURT_E_IO` |
| 2 | usage | The command line is invalid: an unknown option, a missing or invalid value, a missing argument. | |
| 3 | unavailable | The subcommand is not part of this build. | |
| 4 | timeout | The program exceeded its time limit (`E_TIMEOUT`). | `NURT_E_TIMEOUT` |
| 70 | internal | An internal error (`E_INTERNAL`). | `NURT_E_INTERNAL` |
| 130 | cancelled | The program was cancelled (`E_CANCELLED`). | `NURT_E_CANCELLED` |

A report with the status `partial`, `rolled_back` or `failed` takes the code of its error; without an error the code is 1. An engine that cannot be opened, for example because of an invalid configuration, takes the code of its error.

Messages of the command line are written to the standard error output in the form `nurt: <message>`. A language error is written as `<code>: <message>` followed by the ready replacements of the program.
