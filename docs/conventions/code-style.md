<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Code style

## Toolchain and formatting

- Rust edition 2024. The toolchain is pinned in `rust-toolchain.toml`; the
  minimum supported Rust version is the `rust-version` of the workspace.
- Code is formatted with `rustfmt` and the settings in `rustfmt.toml`
  (maximum line width 100, Unix line endings). `cargo fmt --all --check` must
  pass.
- Text files use UTF-8 and LF line endings (`.editorconfig`, `.gitattributes`).

## Lints

Lints are configured in `[workspace.lints]` of the root `Cargo.toml` and in
`clippy.toml`. Every crate inherits them with `[lints] workspace = true`.
Continuous integration treats every warning as an error:

```sh
cargo clippy --workspace --all-targets --locked -- -D warnings
```

A lint is suppressed only on the smallest possible item, with a reason that
explains why the code is correct: `#[expect(lint, reason = "...")]`, or
`#[allow(lint, reason = "...")]` where the lint does not fire on every
target. An `#[allow(...)]` without a `reason` argument is preceded by a
comment that states the reason.

## Documentation

- Library crates declare `#![deny(missing_docs)]`; every public item has
  rustdoc documentation.
- The first paragraph is one sentence that states what the item is or does.
  Sections `# Errors`, `# Panics` (tests and examples only) and `# Safety`
  are used where they apply.
- Examples in documentation compile and run as documentation tests.
- Documentation is built without warnings:
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`.

## Errors and panics

- Production code does not call `unwrap`, `expect`, `panic!`, `todo!` or
  `unimplemented!`, and does not index slices in a way that can panic on
  external input. Fallible operations return `Result` with the error types of
  the `nurt-contracts` crate; each error carries a stable error code.
- Tests may use `unwrap`, `expect` and `panic!`. Integration test files in
  `tests/` start with
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test code")]`.
- Error messages are short and factual, start with a lowercase letter, have no
  period at the end, and state what to correct.

## Unsafe code

- `#![forbid(unsafe_code)]` in every crate, except crates that call operating
  system or foreign interfaces by definition (the C interface, the file system
  layer, the system information layer).
- In those crates every `unsafe` block is as small as possible and is
  preceded by a `// SAFETY:` comment that states why the operation is sound.

## Structure

- Cognitive complexity of a function is at most 15 (`clippy.toml`).
- Items are private unless they are part of the interface of the crate;
  `pub(crate)` is used for items shared within a crate.
- Crates depend on each other only as declared in the crate graph of the
  architecture documentation; shared types live in `nurt-contracts`.
- Platform-specific code is isolated behind `cfg` attributes in dedicated
  modules, so that every crate builds on Linux, macOS and Windows.

- Merged code contains no dead code, no placeholder implementations
  (`todo!`, `unimplemented!`, empty bodies) and no abstractions beyond the
  specification.
- Literal numbers with a meaning are named constants.

## Comments and output

- A comment states what the code does not show: an invariant, the reason for
  a non-obvious decision, a reference to a standard, an RFC or a documented
  behaviour of the operating system. Comments that restate the code or list
  its steps are not written.
- Comments do not describe the history of the code or address a reviewer.
  Commented-out code is not committed.
- Unfinished-work markers are not committed; open work is tracked in issues.
- Library code does not print to standard output or standard error; the
  `dbg!`, `print!` and `eprint!` families are reserved for the command line
  program and for tests. Diagnostics use `tracing`.

## Language

Code, comments, documentation, messages of the program and commit messages
are written in English, in a formal, technical and impersonal register:

- short, declarative, unambiguous sentences; the terminology of the
  specification, used consistently;
- no promotional or evaluative adjectives, no hedging, no phrases addressed to
  the reader, no exclamations, rhetorical questions, emoji or decorative
  symbols;
- no headings or introductions that repeat the content of the document;
- emphasis and lists only where a sentence is not sufficient.

The `style` group of `tools/ci/repo-hygiene.sh` checks these rules where they
can be detected automatically.

## Tests

- Test names describe the behaviour under test, for example
  `rejects_path_outside_root`.
- Test data is minimal, and its purpose is stated.
- Unit tests live in a `tests` module of the file; integration tests in the
  `tests/` directory of the crate.
- Tests are deterministic and self-contained: they create files in temporary
  directories (`tempfile`), use paths relative to the test directory, and do
  not depend on the time zone, the locale or the user account of the machine.
- Tests do not access the network. Tests that need a network or an external
  program are marked `#[ignore]` with a reason and are run explicitly.
- Golden files (expected outputs) are stored in `spec/golden/` and are
  compared as values, not as text; they are regenerated only deliberately.

## Examples and test data

Example paths use `/home/user` and `C:/Users/user`; example domains use
`example.com`, `example.org` and `example.net`; example IPv4 addresses use the
documentation ranges `192.0.2.0/24`, `198.51.100.0/24` and `203.0.113.0/24`.
