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

### Added

- `create` makes named pipes and device nodes on Linux and macOS: `type: fifo
  mode: "0660"` and `type: device kind: char|block major: <int> minor: <int>`
  (a device node needs `privilege: elevated`). On Windows both fail with
  `E_OS_ONLY`. Both are journaled as created files, and `undo` removes them.
  `info` and `list` report the types `fifo` and `device`; a device node has
  the fields `major` and `minor`.

### Changed

- `sql` has no refusals. A statement that changes anything opens the database
  for writing and journals the file and its `-wal`, `-shm` and `-journal` files
  first; `write` is optional, and `write: false` keeps the call read-only.
  `BEGIN`, `COMMIT` and `END` merge with the transaction of the call,
  `ROLLBACK` ends it with `E_ROLLED_BACK`, `ATTACH` and `DETACH` are allowed
  with every attached file journaled before the first change, `VACUUM INTO` is
  journaled as a created file, `VACUUM` runs after the other statements,
  `load_extension` is allowed (its effects are reported as unknown and
  irreversible) and the pragmas `temp_store_directory` and
  `data_store_directory` are allowed (reported as effects on the whole Nurt
  process, which `undo` does not reset). The file of `ATTACH` and `VACUUM INTO`
  is resolved like every path argument (`E_PATH_RELATIVE` without `base`),
  `file:` URIs are read to their path, the call turns the defensive mode of
  SQLite off when it writes, a preview lists the files of `ATTACH` and
  `VACUUM INTO`, the other database files appear as changes of the report, and
  the kind of each statement is read from its own text.
- Control constructs (language brief, sections 1 to 6 and 8): `when` with
  `otherwise when` and `otherwise`; the loops `each`, `repeat until` and
  `while` with `break`, `continue`, `parallel`, `atomic: false`, `limit` and
  the configuration keys `loop.max_iterations` and `loop.max_time`; `steps`
  for several changes in one call; functions with `define` and the key
  `function.max_depth`; script parameters with `params`, the command
  `nurt run <file> name: value` with `--help`, and the member `params` of the
  request. Each construct is one journal entry, all or nothing, previewed
  without a change and checked before any effect; the report lists its
  `parts`. A parameter of type `bool` written without a value is `true`.
- Syntax tree version 1.1 (`nurt-ast/1` and `nurt-bound/1`): the statements
  `when`, `each`, `repeat`, `while`, `break`, `continue`, `steps`, `define`
  and `params` and the pipeline head `call`. Documents of version 1.0 are read
  unchanged. The grammar is in `spec/grammar.md`.
- Changes of the API of `nurt-syntax`: `lex::Checkpoint` is no longer `Copy`
  (it keeps the open blocks of the lexer); `MutationTx` gains the methods
  `allow_incremental`, `savepoint`, `release_savepoint` and
  `rollback_to_savepoint`, with defaults that keep the old behaviour; `Request`
  gains the member `params`; `Report` gains `parts` and `parts_total`.
- `MutationTx::allow_incremental`: a transaction of the journal may preserve
  further targets after its first change, so that a construct is one entry.

## [0.1.1] - 2026-11-27

### Added

- Privilege elevation: the call parameters `privilege` (`account`, `elevated`,
  `auto`) and `as_user`, written on a stage, given in the request or on the
  command line (`--privilege`, `--as-user`); the elevation backends `self`,
  `sudo`, `doas` and `run0` (the backend `broker` is reported as
  `not_installed` until the service `nurt-elevation`); the configuration
  section `[privilege]`; elevated entries in the system journal with
  references in the journal of the account; and undo of elevated changes and
  of changes to packages and services by inverse operations.

### Changed

- A `nurt` process that runs as the administrator keeps its journal, results
  and candidates in the system state directory (`/var/lib/nurt`,
  `/Library/Application Support/Nurt`, `%ProgramData%\Nurt`) instead of the
  state directory of that account; entries made earlier as `root` are not shown
  there.
- `sudo`, `doas` and `run0` in bash lines are translated into `privilege:
  elevated` or `as_user` instead of being rejected; `sudo -i` and `su` end with
  `E_AMBIGUOUS`.
- `E_PRIVILEGE` means that no elevation backend ran the operation or that the
  system denied the elevation; it carries `backend` and `next_steps`.

### Security

- The Git library `gix` and its sub-crates are updated to the 0.91 generation,
  which no longer depends on `heapless` 0.8 (nine advisories about unsound code,
  RUSTSEC-2026-0574 to RUSTSEC-2026-0582).
- The unused `netstat2` dependency and the AVIF encoder of the `image` crate
  (`ravif`, `rav1e`) are removed, which removes the unmaintained `paste` macro
  crate (RUSTSEC-2024-0436). Reading of AVIF files was never enabled.
- OpenPGP signatures of commits and tags are made and checked by GnuPG instead
  of the `pgp` crate, which removes `rsa` 0.9 (RUSTSEC-2023-0071) and
  `crypto-bigint` 0.5 (RUSTSEC-2026-0387) from the dependency graph. The
  program named by `gpg.openpgp.program` or `gpg.program` (`gpg` by default)
  runs through the program service; `git.signing.key_file` is imported into a
  temporary GnuPG home directory. A missing or failing program refuses the
  signature with `E_GIT_SIGN` (problems `program_unavailable` and
  `program_failed`); verification without the program reports the status
  `unsupported`. SSH signatures are unchanged and still made in the process.
- The text extraction of `pdf-extract` 0.12.1 is a crate of the workspace
  (`crates/nurt-pdf-extract`, MIT license) built on `lopdf` 0.47, which no
  longer needs the unmaintained `ttf-parser` crate (RUSTSEC-2026-0192).
- The `chrono` feature is off for `croner`, `lopdf`, `rmcp` (updated to 3.12)
  and, through it, `schemars`. `chrono` stays in the dependency graph only
  through `macos-unifiedlogs` and `russh-util` (RUSTSEC-2026-0543), for which
  `deny.toml` records the reason.
- The lock files of the fuzz targets are regenerated.

### Fixed

- The code review of 0.1.0 corrected the defects found in every crate of the
  workspace, the bindings, the packaging and the documentation; the main ones:
  - requests of `nurt serve` resolve relative paths against `options.base`;
  - the journal of a volume shared by several accounts gives each operation
    a number of its own, and history shows the entries of the account only;
  - `check` reports the types of fields, `in`, arithmetic and the arguments of
    functions;
  - a corrected name of a changing command is offered as a candidate
    (`E_AMBIGUOUS`) and is not applied by itself;
  - paths that are not valid Unicode keep their native bytes beside their
    display form in reports and journal records (the records `git_status` and
    `git_conflict` and the hunks of `git.show` included, and `query` reads an
    entry of such a name), and report values stay on one line in the text
    views;
  - `registry:` paths are accepted by the file commands on Windows;
  - `nurt fmt` keeps the comments of the program;
  - `nurt schema tool` prints the tool definition for the platforms of the
    integrators, as documented;
  - `service install` writes the user unit where the service manager reads it.
- Defects found by running the whole test suite on Windows and on real Linux
  volumes:
  - a file of the lower layer of an overlay belongs to the volume of the
    overlay, so that the journal preserves it on the right volume;
  - an entry preserved by copy keeps its permissions and times;
  - `clone` on Windows writes the configuration as Git for Windows does (line
    ends, `core.symlinks` from the system configuration) and stores a local
    URL with forward slashes;
  - credential helpers, `gpg.ssh.allowedSignersFile` and GnuPG of Git for
    Windows (an MSYS program, which gets `/c/…` paths) work on Windows;
  - `push` from a second worktree on Windows finds the common directory of the
    repository;
  - the change report of a command that writes through the journal names the
    kind of every changed entry on Windows as well.
  - a cancelled `sql` change stops also when the cancellation comes between
    two statements of its transaction; before, such a cancellation was lost and
    a statement without end kept running.
  - concurrent refreshes of one symbol index on Windows keep the rule "the
    last rename wins"; before, a refresh could fail to store the index while
    another process replaced the same file.

### Changed

- The full-scale undo test of one million removed, moved, replaced or
  extracted files allows an undo of up to six times the duration of the
  operation (three times before); the undo of one million removed files takes
  5.3 times the removal.

- `run.spool_max` defaults to `1gib` instead of `unlimited`: a program that
  writes without end no longer fills the disk; output beyond the limit is
  counted, hashed and reported with the `spool_limit` note.
- Journal: an operation with many small files (a `replace` over thousands of
  files and its `undo`) is several times faster and its entry is proportional to
  the changed content. In a `preserve` call with at least 64 targets the
  previous content of small files (up to 4096 bytes) is kept in the manifest
  instead of a block and a metadata file per file; a transaction with at least
  64 targets flushes its new files and directories with one call before the
  commit instead of one flush per file; the volume of a directory is described
  once per directory. Manifests of operations with at least 256 targets have
  format version 2 (packed target records, same digest rule); journals written
  by 0.1.0 are read unchanged, and a reader of version 1 rejects version 2.
  The items of an undo or redo entry are packed with the targets, so that the
  entry of an undo of thousands of files is about seven times smaller.
- `run`: the response is short by default. The text of all streams together
  fits in `max_bytes` (new parameter, key `output.max_bytes`, 4096 bytes, `0`
  is no limit), filled with the errors found, the end of standard error, the
  end of standard output and the beginning; a line longer than 400 characters
  is cut and ends with `… [+N characters]`. `dedupe` is on by default and also
  collapses lines that differ only in numbers, versions or the names after
  `Compiling` and similar words. After exit code 0, `show: auto` returns the
  last 5 lines and the field `digest` instead of the first 20 and last 40
  lines. After a failure with more than 20 lines of output, the output of
  `cargo`/`rustc` and `pytest` is recognised: the field `diagnostics` holds
  the numbers of errors and warnings and the first 10 distinct errors with
  file, line, column, code and message, and the text keeps the last 10 lines.
  A cut response carries `read_hint`, a sentence with `read` and `search`
  commands that read the rest from the output file. Pass `dedupe: false`,
  `max_bytes: 0` or `head` and `tail` to get the earlier behaviour.
- Linux: the executable and the library link liblzma statically; the only
  shared libraries are `libc.so.6`, `libm.so.6`, `libgcc_s.so.1` and the
  dynamic loader, so the program runs on a system without `liblzma.so.5`. The
  `.deb` package no longer depends on `liblzma5`.
- `nurt-service` compiles for the target `x86_64-unknown-linux-musl`
  (`sched_param` and `pthread_t` differ from glibc), and the release procedure
  builds the static musl archive `nurt-<version>-x86_64-unknown-linux-musl`.
- The start banner of an interactive session shows the mark of Danaco Nurt in
  half blocks (ASCII on terminals without UTF-8) in its two colours, instead of
  the name in letters.
- The colours of the mark in the terminal follow the production identity:
  silver `#DDE1E6` (palette 254) and gold `#C9A15A` (palette 179).
- The Windows executable and the documentation site use the new icon and
  favicon.

### Known limitations

- No macOS archive is built; the code for macOS passes `cargo check` for
  `x86_64-apple-darwin` and `aarch64-apple-darwin`, but it is not run.

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
  characters in silver and gold; the prompt is gold with a silver indicator and
  the cursor is a blinking underscore while a line is typed. The highlighting of
  a typed line uses gold for commands, silver for the pipe and cyan for strings.
  A terminal that reports 24-bit colour shows exactly the colours of the mark,
  any other terminal the nearest colours of the 256-colour palette.
- The visual identity in `branding`: the mark of two chevrons turned by 180
  degrees, the application icons, the favicon of the documentation site, the
  lockups with the name drawn in letters of its own, a render of the mark and the identity guidelines.
- `nurt.exe` carries version information and a manifest with the execution
  level `asInvoker`, long path support and the UTF-8 code page, and the icon
  of the product (`crates/nurt-cli/windows/nurt.ico`).
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
- Native extensions are not available; extensions are WebAssembly modules.
- The subcommand `journal` offers only `salvage`; the other journal operations
  are commands of the language.
- The recovery time of 5 seconds for a journal of one million entries is not
  confirmed: the last measurement was 6.2 seconds, before later corrections.
- The certificate that signs `nurt.exe` is issued by Danaco Holding Group
  Sp. z o.o. itself, not by a public certification authority. Windows reports
  the publisher as unverified until a certificate of a public authority replaces
  it.

[Unreleased]: https://github.com/DanacoOfficial/danaco-nurt/compare/v0.1.1...main
[0.1.1]: https://github.com/DanacoOfficial/danaco-nurt/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/DanacoOfficial/danaco-nurt/releases/tag/v0.1.0
