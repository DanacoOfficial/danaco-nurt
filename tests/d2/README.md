<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Journal test suites

Integration test targets of the crate `nurt-crashfs`. Each directory is one
target declared in `crates/nurt-crashfs/Cargo.toml`.

| Target | Directory | Scope |
|---|---|---|
| `crash` | `tests/d2/crash` | Crash points, fault injection and recovery of the journal. |
| `coverage` | `tests/d2/coverage` | Coverage table `tables/journal-coverage.toml` against the command catalog; undo and redo round trip of every row. |
| `retention` | `tests/d2/retention` | Step, age and size limits, space reserve, precedence of configuration levels, absence of expiry times in results. |
| `property` | `tests/d2/property` | Random sequences of operations, undos, limit changes, clock steps and restarts against a reference model. |
| `perf` | `tests/d2/perf` | Latency of journaled writes and counts of file system operations. |
| `volumes` | `tests/d2/volumes` | Prepared file-backed volumes and overlay mounts. |

The directory `tests/d2/coverage/harness` holds the fixture shared by the
targets `coverage`, `retention`, `property`, `perf` and `volumes`: the
journal store assembled over in-memory volumes (`MemFs`), over volumes with
simulated power loss (`CrashFs`) when a test sets a crash seed, or over
mounted directories; tree snapshots with full metadata, a virtual clock and
builders of journal requests.

## Running

```
cargo test -p nurt-crashfs --all-features --test coverage
cargo test -p nurt-crashfs --all-features --test volumes -- --ignored
```

Tests that need a component outside the crate graph of the build, a prepared
volume or a run at full scale are marked `#[ignore]`; the reason names the
missing component or condition.

## Environment

| Variable | Volume |
|---|---|
| `NURT_TEST_EXT4_DIR` | ext4 of 1 GiB from a file |
| `NURT_TEST_EXT4_SMALL_DIR` | ext4 of 64 MiB from a file |
| `NURT_TEST_XFS_DIR` | XFS from a file |
| `NURT_TEST_BTRFS_DIR` | btrfs from a file |
| `NURT_TEST_OVERLAY_DIR` | overlay mount whose directory `lower` comes from the lower layer |
| `NURT_TEST_OVERLAY_SMALL_DIR` | overlay mount with a 64 MiB upper layer and a directory `lower` of 48 MiB from the lower layer |
| `NURT_TEST_TMPFS_DIR` | tmpfs |

A volume test run without its variable fails and names the variable. The
journal tests never use tmpfs as a journaled volume: the journal is inactive
there.
