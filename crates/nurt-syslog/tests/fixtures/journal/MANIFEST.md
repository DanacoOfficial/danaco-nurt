<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Journal test corpus

The repository stores no journal file written by systemd. The tests of the
journal reader use two kinds of input:

| Input | Origin | License |
|---|---|---|
| Files produced by `tests/support/journal_writer.rs` during the tests | Written from the published journal file format; deterministic for equal options and entries | Apache-2.0, as the repository |
| Files produced by systemd in a continuous integration job | Written by the systemd of the job runner; not committed | not distributed |

## Writer variants

The writer covers every combination used by the tests: regular and compact
mode, lookup3 and keyed hash, payloads stored uncompressed or compressed with
XZ, LZ4 or zstd, the states `OFFLINE`, `ONLINE` and `ARCHIVED`, files whose
header lags behind their objects (an active file), rotation into several files,
and files cut at any byte.

## Files written by systemd

A job on a runner with systemd writes a file with `systemd-journal-remote`
from export-format input and compares the reader with `journalctl`:

```sh
printf '__REALTIME_TIMESTAMP=1700000000000000\nMESSAGE=hello\nPRIORITY=6\n\n' \
  | /usr/lib/systemd/systemd-journal-remote -o "$RUNNER_TEMP/sample.journal" -
cargo test -p nurt-syslog --test journald_differential -- --include-ignored
```

The job records the systemd version (`systemctl --version`) in its log. Files
written this way stay in the temporary directory of the job and are not added
to this directory.
