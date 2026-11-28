<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           CONTRIBUTING.md -->
<!-- Responsibility:   Documents Contributing to Danaco Nurt. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Contributing to Danaco Nurt

Changes are proposed as pull requests and are merged when they satisfy the
rules below. Participants follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Issues

- Search the existing issues before opening a new one.
- For a defect, open an issue with the version, the operating system, a minimal
  program that reproduces the problem and the actual and expected results.
- For a new command, parameter, record type or a change of the language, open
  an issue describing the use case before writing code. Changes of the
  language, the syntax tree, the report format and the C interface follow the
  compatibility rules of the specification in the `spec` directory.
- Security vulnerabilities are reported privately, as described in
  [SECURITY.md](SECURITY.md).

The language of code, comments, documentation, messages of the program,
commit messages, issues and pull requests is English, in a formal and
technical register: short declarative sentences, the terminology of the
specification, no promotional or hedging wording, no emoji. The rules are
described in [docs/conventions/code-style.md](docs/conventions/code-style.md).

## Developer Certificate of Origin

Contributions are accepted under the Developer Certificate of Origin (DCO),
version 1.1, published at <https://developercertificate.org>. By signing off a
commit you certify the following:

```text
Developer's Certificate of Origin 1.1

By making a contribution to this project, I certify that:

(a) The contribution was created in whole or in part by me and I
    have the right to submit it under the open source license
    indicated in the file; or

(b) The contribution is based upon previous work that, to the best
    of my knowledge, is covered under an appropriate open source
    license and I have the right under that license to submit that
    work with modifications, whether created in whole or in part
    by me, under the same open source license (unless I am
    permitted to submit under a different license), as indicated
    in the file; or

(c) The contribution was provided directly to me by some other
    person who certified (a), (b) or (c) and I have not modified
    it.

(d) I understand and agree that this project and the contribution
    are public and that a record of the contribution (including all
    personal information I submit with it, including my sign-off) is
    maintained indefinitely and may be redistributed consistent with
    this project or the open source license(s) involved.
```

Every commit must contain a `Signed-off-by` trailer with the name and e-mail
address of the author, matching the author of the commit:

```text
Signed-off-by: Jane Doe <jane.doe@example.com>
```

`git commit --signoff` (`-s`) adds the trailer from the `user.name` and
`user.email` settings of Git. Contributions are licensed under the
[Apache License, Version 2.0](LICENSE), as stated in Section 5 of the License.

## Commit messages

Commit messages follow [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/):

```text
<type>(<scope>)[!]: <summary>

[body]

[footers]
```

- `type` is one of `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`,
  `ci`, `style`, `chore`, `revert`.
- `scope` is the name of the affected crate without the `nurt-` prefix (for
  example `contracts`, `journal`, `git-ops`) or one of the repository areas
  `workspace`, `deps`, `ci`, `spec`, `catalog`, `tables`, `docs`, `tools`,
  `release`, `repo`, `website`.
- `summary` is written in the imperative mood, starts with a lowercase letter,
  does not end with a period and is at most 72 characters long.
- A breaking change is marked with `!` after the scope and described in a
  `BREAKING CHANGE:` footer.

The full rules, with examples, are in
[docs/conventions/commits.md](docs/conventions/commits.md).

## File headers

The license of the product is stated only in the [LICENSE](LICENSE) file.
Source files do not carry per-file license identifier lines. Every source file
in a format with comments starts with the producer header, written in the
comment syntax of the format:

```rust
// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
```

Files in formats without comments (for example JSON) have no header. Texts
reproduced from third parties, such as `LICENSE` and `CODE_OF_CONDUCT.md`, are
kept verbatim. Headers missing from a pull request are completed by the
maintainers before merge. Details are in
[docs/conventions/licensing.md](docs/conventions/licensing.md).

## Code standard

The rules are summarised here and described in
[docs/conventions/code-style.md](docs/conventions/code-style.md) and
[docs/conventions/naming.md](docs/conventions/naming.md).

- Rust edition 2024, with the toolchain pinned in `rust-toolchain.toml`;
  formatting with `rustfmt` and the settings in `rustfmt.toml`.
- Every public item has rustdoc documentation; library crates deny
  `missing_docs`.
- No `unwrap`, `expect` or `panic!` in production code; errors are returned as
  the error types defined by the `nurt-contracts` crate.
- `unsafe` code is forbidden outside the crates that call operating system
  interfaces by definition; every `unsafe` block there has a `// SAFETY:`
  comment.
- No commented-out code, no diagnostic printing, no unfinished-work markers
  and no placeholder macros (`todo!`, `unimplemented!`) in merged code.
- Comments state what the code does not show: invariants, reasons for
  non-obvious decisions, references to standards. Comments do not describe the
  history of the code.
- Every `#[allow(...)]` states its reason in a `reason = "..."` argument or in
  a comment on the same or the preceding line.
- Cognitive complexity of a function is at most 15; named constants replace
  literal numbers with a meaning.
- Tests are self-contained and deterministic: temporary directories, no network
  access (integration tests that need a network are ignored by default), no
  absolute paths of the machine that runs them.
- Example paths in documentation and tests use `/home/user`, `C:/Users/user`
  and the `example.com` domain.
- New dependencies are declared in `[workspace.dependencies]` of the root
  `Cargo.toml`, must use a license from the allowed list in
  [docs/conventions/licensing.md](docs/conventions/licensing.md), and are
  justified in the pull request.

## Checks before a pull request

A pull request is merged only when all of the following succeed. Continuous
integration runs them on Linux, macOS and Windows.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo deny --locked check
cargo nextest run --workspace --locked      # or: cargo test --workspace --locked
cargo test --workspace --doc --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
bash tools/ci/repo-hygiene.sh
bash tools/ci/check-commits.sh origin/main..HEAD
```

`tools/ci/repo-hygiene.sh --list` lists the checks of repository content,
language and style, and file headers.

Property tests (`proptest`) generate a few hundred cases per property in a
regular run, so that every test of `nurt-syntax` takes less than 30 seconds in a
debug build. The scheduled nightly run of `ci.yml` sets `NURT_PROPTEST_FULL=1`
and generates the full number of cases (100 000 for most properties).
`PROPTEST_CASES=<n>` overrides the number in both runs:

```sh
NURT_PROPTEST_FULL=1 cargo nextest run -p nurt-syntax -E 'binary(cbor_roundtrip) or binary(roundtrip_print)'
```

Test coverage is measured with
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov). The minimum line
coverage is 80% for the workspace and 90% for the crates `nurt-contracts`,
`nurt-syntax`, `nurt-correct`, `nurt-journal`, `nurt-fs` and `nurt-ffi`:

```sh
cargo llvm-cov nextest --workspace --locked --fail-under-lines 80
```

## Pull requests

- Keep a pull request focused on one change; split unrelated changes.
- The description states the motivation, the change and the tests; the pull
  request template lists the required items.
- Add or update tests and documentation together with the code. User-visible
  changes are recorded in the `Unreleased` section of [CHANGELOG.md](CHANGELOG.md).
- Do not commit build outputs, logs, editor files or local configuration.
- A maintainer reviews every pull request. Commits are merged as they are, so
  each commit must satisfy the commit message and sign-off rules.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
