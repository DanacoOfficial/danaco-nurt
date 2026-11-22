<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Changelog

All notable changes to Danaco Nurt are documented in this file.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
and the project adheres to [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).
The language, the syntax tree, the command catalog and the C interface carry
their own version numbers, described in the specification.

## [Unreleased]

## [0.1.0] - 2026-11-22

First release.

### Added

- Cargo workspace with the crates of the engine, the command line program, the
  C interface and the test support libraries.
- Contract crate `nurt-contracts` with the shared interfaces and data formats.
- Command catalog and specification documents.
- Language: lexer, parser with three syntax variants, correction of programs,
  binding against the command catalog, printing, help and descriptions in
  several layers, migration of Bash and PowerShell programs and a language
  server.
- Engine: planning, checking and execution of programs, structured reports of
  effects in text, JSON and Markdown form, pagination, sessions, candidates for
  ambiguous programs and cancellation.
- Undo journal: every change to files, provider values and repository state
  that the program can preserve is recorded and can be reviewed, undone and
  redone; recovery after a crash; retention limits; history queries. Effects
  that cannot be preserved are recorded as irreversible and reported
  separately.
- Command groups: files, text, archives, notebooks, structured documents,
  source code analysis with a language server client, processes, tests and
  analysis, version control implemented in Rust, system, network, scheduling,
  services and programs.
- Command line program with the subcommands `exec`, `check`, `script`, `repl`,
  `serve`, `journal`, `mcp`, `lsp`, `migrate`, `service` and `ext`; the Model
  Context Protocol server; the C interface (`libnurt`, `nurt.h`); bindings for
  several languages; a conformance suite and benchmarks.
- Packaging for Linux, macOS and Windows, release automation and integrations
  for editors and containers.
- License, notice, contribution guide, security policy and code of conduct.
- OpenPGP signature of `checksums.txt` and of the release tag, the Authenticode
  signature of `nurt.exe`, and the public key and certificate in
  `packaging/release`; see `SECURITY.md` for the verification.
- Dependency policy for `cargo-deny`, continuous integration on Linux, macOS
  and Windows, and a repository hygiene check.
- The banner of an interactive session draws the name of the product with
  characters; the cursor is a blinking underscore while a line is typed.
- `nurt.exe` carries version information and a manifest with the execution
  level `asInvoker`, long path support and the UTF-8 code page. The build
  embeds an icon when the file `crates/nurt-cli/windows/nurt.ico` exists.
- `install.ps1` installs for the current user by default and for all users into
  `%ProgramFiles%\Nurt` with `NURT_SCOPE=machine` from a terminal that runs as
  administrator; it installs the documentation next to the executable.
- The archives hold the documentation site, the completion scripts for five
  shells and, on Linux, the manual pages; `packaging/release/pack.py` assembles
  them reproducibly.
- The shared library `libnurt.so.1` records the SONAME `libnurt.so.1`.
- Every commit of the `main` branch is signed with a dedicated OpenPGP key; see
  `SECURITY.md`.

### Known limitations

- The journal is active on volumes that keep a journal; changes on volumes held
  in memory, such as `tmpfs`, are reported as irreversible.
- This release has archives for Linux (glibc, x86-64) and Windows (MSVC,
  x86-64). The Windows archive and `install.ps1` were run on Windows 11 only,
  and the registry provider is available only on Windows. The code for macOS
  exists, but no macOS archive is built. `install.sh` takes the glibc archive of
  Linux because the release has no musl archive.
- `nurt.exe` has no icon yet; the icon of the product is not designed.
- Native extensions are not available; extensions are WebAssembly modules.
- The subcommand `journal` offers only `salvage`; the other journal operations
  are commands of the language.
- The recovery time of 5 seconds for a journal of one million entries is not
  confirmed: the last measurement was 6.2 seconds, before later corrections.
- The certificate that signs `nurt.exe` is issued by Danaco Holding Group
  Sp. z o.o. itself, not by a public certification authority. Windows reports
  the publisher as unverified until a certificate of a public authority replaces
  it.

[Unreleased]: https://github.com/DanacoOfficial/nurt/compare/v0.1.0...main
[0.1.0]: https://github.com/DanacoOfficial/nurt/releases/tag/v0.1.0
