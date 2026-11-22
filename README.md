<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Danaco Nurt

Danaco Nurt — command console with typed pipelines and per-operation undo.

`nurt` (pronounced *NOORT*) is a command console with its own language for
working with files, text, structured documents, source code, version control,
processes and the operating system. Programs are pipelines of typed records;
each program is checked before it runs, and each run is described by a
structured report of its effects.

## Status

The project is in early development. The language, the syntax tree, the
command catalog, the report format and the library interfaces are being
specified, and the implementation is not yet functional. No release has been
published. Interfaces may change without notice until the first stable release.

## Design

- One executable per operating system (`nurt`, `nurt.exe`) for Linux, macOS
  and Windows, without runtime dependencies.
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

The language is under specification. The following program lists the 20
largest Go source files changed during the last day:

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
git clone https://github.com/DanacoOfficial/nurt.git
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
