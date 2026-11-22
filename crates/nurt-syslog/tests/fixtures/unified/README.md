<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Unified log fixtures

`entries.json` holds log messages in the shape of `OSLogEntryLog`: `date` in
seconds since the Unix epoch as `NSDate` gives it, `level` as the raw value of
`OSLogEntryLog.Level` (0 undefined to 5 fault) and the other properties under
their names in the framework. The entries are ordered newest first. The
processes, subsystems and messages are invented for the tests.
