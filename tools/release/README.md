<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           tools/release/README.md -->
<!-- Responsibility:   Documents Release scripts. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Release scripts

Scripts that build, check and verify a release. They run from any directory and locate the repository from their own path.

| Script | Purpose |
|---|---|
| `scenarios.sh <nurt> <results-dir>` | Repeats the functional scenarios T01–T21 and 2.11 against a built executable. |
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
tools/release/build-release.sh 0.1.2 /tmp/release-0.1.2
tools/release/build-release.sh 0.1.2 /tmp/release-0.1.2 --sign /path/to/gnupg-home KEYID
```

Steps: build of `nurt-cli` and `nurt-ffi` for the targets of the table below; `nurt-docgen man`, `completions` and `site`; `packaging/release/pack.py` for each target; `checksums.txt`; with `--sign` a detached armored signature `checksums.txt.asc` and `gpg --verify`; then, for the target of the host only, `nurt version` and `nurt doctor` from the unpacked archive of the program and `examples/c/hello.c` built (flags from `pkg-config`) and run against the unpacked archive of the library. On an x86_64 Linux host with a glibc target the static musl library `libnurt.a` of `x86_64-unknown-linux-musl` is checked as well (`pkg-config --static`, `-static`, C compiler `$CC_x86_64_unknown_linux_musl` or `packaging/release/zig-cc-musl.sh`).

| Target | Built | C compiler and linker |
|---|---|---|
| `x86_64-unknown-linux-gnu` | always | `cc` of the host |
| `x86_64-unknown-linux-musl` | `NURT_RELEASE_MUSL=auto` (target installed), `1` or `0` | compiler `packaging/release/zig-cc-musl.sh` (zig cc), linker `cc` |
| `aarch64-unknown-linux-gnu` | `NURT_RELEASE_AARCH64=auto` (both aarch64 Linux targets installed and zig available), `1` or `0` | `packaging/release/zig-cc-aarch64-linux-gnu.sh` (`-target aarch64-linux-gnu.2.31`) |
| `aarch64-unknown-linux-musl` | together with the previous one | `packaging/release/zig-cc-aarch64-linux-musl.sh` |
| `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc` | unless `--no-windows` | `cargo xwin build --target <target>` |

For the targets built with zig the script sets `CC_<target>`, `AR_<target>` (`packaging/release/zig-ar.sh`, `zig ar`) and `CFLAGS_<target>=-fno-sanitize=undefined` and, for aarch64, `CARGO_TARGET_<TARGET>_LINKER`, unless the caller has set them; `ZIG` names the zig executable (`zig` by default). The wrappers drop the option `--target=...` of the `cc` crate and, for aarch64, the linker options `-Wl,--fix-cortex-a53-843419` and `-Wl,-O1`, which zig rejects or warns about.

The layout has two archives per target: `nurt-<version>-<target>` with the executable and `nurt-lib-<version>-<target>` with the shared library (`libnurt.so.1`, `libnurt.1.dylib` or `nurt.dll`) and the header. On musl targets the library is static: the build passes `libnurt_ffi.a`, and the archive holds `libnurt.a`, `nurt.h` and a `nurt.pc` whose `Libs.private` lists the libraries of a static link. One call of `pack.py` with `--library` writes both archives. Both archives are listed in `checksums.txt`.

The environment of the build (`CARGO_TARGET_DIR`, `RUSTUP_TOOLCHAIN`, `RUSTC_WRAPPER`) is taken from the caller. The output directory must be empty or absent. A key is never read from a default location: `--sign` requires an explicit GnuPG home.

## clean-linux-check.sh

```sh
tools/release/clean-linux-check.sh /tmp/release-0.1.2/nurt-0.1.2-x86_64-unknown-linux-gnu.tar.gz
```

The archive is mounted read-only into a container of `debian:stable-slim` (glibc archive) or `alpine:latest` (musl archive; checked as well when it lies next to the glibc archive). The container has no network. It unpacks the archive, runs `nurt version` and `nurt doctor`, overwrites a file, runs `undo` and compares the file with its original content. `NURT_CONTAINER_ENGINE` selects `docker` or `podman`; without it the first engine that answers `info` is used (rootless `podman` without subordinate user identifiers cannot unpack the images). The images must be present or pullable by the engine.

## Tests

`python3 -I -m unittest discover -s packaging/tests -p 'test_release_scripts.py'` tests the parsing of the scenario results and the argument handling of the scripts.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
