<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Release scripts

Scripts that build, check and verify a release. They run from any directory and locate the repository from their own path.

| Script | Purpose |
|---|---|
| `scenarios.sh <nurt> <results-dir>` | Repeats the functional scenarios T01–T19 and 2.11 against a built executable. |
| `build-release.sh <version> <output-dir> [--sign <gnupg-home> <key-id>]` | Builds, packs, checksums and optionally signs the release archives. |
| `clean-linux-check.sh <archive.tar.gz>` | Runs a Linux archive in a container of a minimal image. |
| `scenario_table.py <results-file>` | Renders the result lines of `scenarios.sh` as a table; used by `scenarios.sh`. |

## scenarios.sh

```sh
cargo build --release -p nurt-cli
tools/release/scenarios.sh target/release/nurt /tmp/nurt-scenarios
```

Each scenario runs in its own empty temporary directory with its own `HOME`, `XDG_STATE_HOME` and `XDG_CONFIG_HOME`. The temporary directories are created under `$TMPDIR` (the volume of that directory shares one journal, which scenario T01 relies on) and are removed at the end.

The results directory receives `results.tsv` (identifier, status, reason, separated by tabs), `table.txt` (the table printed at the end) and `<id>.log` (the output of every command of the scenario).

| Status | Meaning |
|---|---|
| `PASS` | The behaviour that the fix specifies was observed. |
| `FAIL` | The behaviour differs; the reason names the observed difference. |
| `SKIP` | The scenario needs Windows or macOS (T05, T09 path part, T10). |

Exit code: 0 without a `FAIL`, 1 with at least one `FAIL`, 2 for a usage error. `NURT_SCENARIO_ONLY` (a space-separated list of identifiers) runs a subset; `NURT_SCENARIO_TIMEOUT` sets the limit of one command in seconds (default 120).

## build-release.sh

```sh
tools/release/build-release.sh 0.1.1 /tmp/release-0.1.1
tools/release/build-release.sh 0.1.1 /tmp/release-0.1.1 --sign /path/to/gnupg-home KEYID
```

Steps: build of `nurt-cli` and `nurt-ffi` for `x86_64-unknown-linux-gnu`, for `x86_64-unknown-linux-musl` when the target is installed (`NURT_RELEASE_MUSL=auto`, `1` or `0`) and for `x86_64-pc-windows-msvc` with `cargo xwin build` (skipped with `--no-windows`); `nurt-docgen man`, `completions` and `site`; `packaging/release/pack.py` for each target; `checksums.txt`; with `--sign` a detached armored signature `checksums.txt.asc` and `gpg --verify`; `nurt version` and `nurt doctor` from the unpacked Linux archive.

The archive of the library `nurt-lib` is assembled with `--lib-archive`. The command is the function `lib_pack_command` at the top of the script; changing the call of the executor of the library archive is a change of that function only.

The environment of the build (`CARGO_TARGET_DIR`, `RUSTUP_TOOLCHAIN`, `RUSTC_WRAPPER`) is taken from the caller. The output directory must be empty or absent. A key is never read from a default location: `--sign` requires an explicit GnuPG home.

## clean-linux-check.sh

```sh
tools/release/clean-linux-check.sh /tmp/release-0.1.1/nurt-0.1.1-x86_64-unknown-linux-gnu.tar.gz
```

The archive is mounted read-only into a container of `debian:stable-slim` (glibc archive) or `alpine:latest` (musl archive; checked as well when it lies next to the glibc archive). The container has no network. It unpacks the archive, runs `nurt version` and `nurt doctor`, overwrites a file, runs `undo` and compares the file with its original content. `NURT_CONTAINER_ENGINE` selects `docker` or `podman`; without it the first engine that answers `info` is used (rootless `podman` without subordinate user identifiers cannot unpack the images). The images must be present or pullable by the engine.

## Tests

`python3 -I -m unittest discover -s packaging/tests -p 'test_release_scripts.py'` tests the parsing of the scenario results and the argument handling of the scripts.
