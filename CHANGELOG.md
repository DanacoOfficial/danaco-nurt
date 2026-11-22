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
- Dependency policy for `cargo-deny`, continuous integration on Linux, macOS
  and Windows, and a repository hygiene check.

### Known limitations

- The journal is active on volumes that keep a journal; changes on volumes held
  in memory, such as `tmpfs`, are reported as irreversible.
- Only the Linux archive of the glibc target is built for this release. The
  code for Windows and macOS exists, but it was neither built nor run; the
  registry provider is available only on Windows.
- Native extensions are not available; extensions are WebAssembly modules.
- The subcommand `journal` offers only `salvage`; the other journal operations
  are commands of the language.
- The recovery time of 5 seconds for a journal of one million entries is not
  confirmed: the last measurement was 6.2 seconds, before later corrections.
- The release is not signed.

[Unreleased]: https://github.com/DanacoOfficial/nurt/compare/v0.1.0...main
[0.1.0]: https://github.com/DanacoOfficial/nurt/releases/tag/v0.1.0
