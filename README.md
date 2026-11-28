<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           README.md -->
<!-- Responsibility:   Documents Danaco Nurt. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="branding/svg/lockup-horizontal-color-dark.svg">
  <img src="branding/svg/lockup-horizontal-color-light.svg" alt="Danaco Nurt" width="360">
</picture>

# Danaco Nurt

Danaco Nurt is a command shell and language where every change can be undone,
built for people and AI agents. One tool for Linux, Windows and macOS.

`nurt` (pronounced *NOORT*) is a command shell with its own language for
working with files, text, structured documents, source code, version control,
processes and the operating system. Programs are pipelines of typed records;
each program is checked before it runs, and each run is described by a
structured report of its effects.

## Status

Version 0.1.0 is the first release. The language front end, the engine, the
command catalog, the undo journal, the command line program, the Model Context
Protocol server, the language server and the C interface are implemented and
work together. The release is not stable: the language, the syntax tree, the
command catalog, the report format and the library interfaces may change
between 0.x versions. The known limitations are listed in the
[changelog](CHANGELOG.md).

## Design

- One executable per operating system (`nurt`, `nurt.exe`) for Linux, macOS
  and Windows, with no tools to install besides the system itself. The glibc
  build for Linux links the system libraries `libc.so.6`, `libm.so.6` and
  `libgcc_s.so.1` dynamically (the `.deb` and `.rpm` packages declare them);
  the other libraries are part of the executable;
  the musl build for Linux is linked statically.
- File, text, archive, document, code, version control, network and system
  operations are implemented in the program and do not invoke external tools.
- Stages exchange records with named fields (files, lines, matches, processes)
  instead of unstructured text.
- Names, parameters and types are validated against the command catalog
  before any effect takes place. Errors carry a stable code, a position and a
  correction.
- Each run returns a report of its effects in text, JSON or Markdown form; the
  report schema is published in the `spec` directory.
- File changes are recorded in a journal and can be reviewed and undone.
- The engine is a Rust library with a C interface (`libnurt`, `nurt.h`), used
  by the command line program and by the Model Context Protocol server.

## Example

The following program lists the 20 largest Go source files changed during the
last day:

```text
find "/home/user/project" glob: "**/*.go"
  |> filter size > 1mb and modified > ago(24h)
  |> sort size order: desc
  |> first 20
```

## Building from source

Requirements:

- Rust, installed with [rustup](https://rustup.rs). The required toolchain is
  pinned in `rust-toolchain.toml` and is installed automatically by `rustup`.
- A C compiler (bundled components such as SQLite and tree-sitter grammars are
  compiled from source).

Build:

```sh
git clone https://github.com/DanacoOfficial/danaco-nurt.git
cd nurt
cargo build --release --locked
```

The executable is written to `target/release/nurt` (`target\release\nurt.exe`
on Windows).

Run the test suite:

```sh
cargo test --workspace --locked
```

## Repository layout

| Path | Content |
|---|---|
| `crates/` | Rust crates of the engine, the command line program and the libraries |
| `spec/` | Specification: language, syntax tree, schemas, reports, journal |
| `catalog/` | Command catalog (signatures and descriptions of commands) |
| `docs/` | User and integrator documentation, contribution conventions |
| `tables/` | Data tables used by the engine and its tests |
| `tools/` | Scripts for continuous integration and maintenance |

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before
opening a pull request. All participants are expected to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Security

Do not report security vulnerabilities in public issues. Follow the process
described in [SECURITY.md](SECURITY.md).

## License

Danaco Nurt is licensed under the [Apache License, Version 2.0](LICENSE).
Distributions must include the [NOTICE](NOTICE) file. The License does not
grant permission to use the names "Danaco" or "Danaco Nurt" beyond the
customary description of the origin of the product; see
[docs/conventions/naming.md](docs/conventions/naming.md).

Copyright 2026 Danaco Holding Group Sp. z o.o.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
