<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/conventions/licensing.md -->
<!-- Responsibility:   Documents Licensing conventions. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Licensing conventions

## License of the product

Danaco Nurt is licensed under the Apache License, Version 2.0. The full text
is in [LICENSE](../../LICENSE); the attribution notice is in
[NOTICE](../../NOTICE) and must be included in every distribution (Section
4(d) of the License). Contributions are accepted under the same license and
the Developer Certificate of Origin 1.1 (see
[CONTRIBUTING.md](../../CONTRIBUTING.md)).

## File headers

The license of the product is stated only in the `LICENSE` file. Source files
do not carry per-file license identifier lines (SPDX identifiers or license
notices).

Every source file in a format with comments starts with the producer header,
three lines in the comment syntax of the format:

```text
Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
https://github.com/DanacoOfficial | support@danaco-group.pl
```

| Format | Comment syntax of the header |
|---|---|
| Rust, C, C header, JavaScript, TypeScript | each line prefixed with `// ` |
| TOML, YAML, shell, Python, PowerShell, `.gitignore`, `.gitattributes`, `.editorconfig` | each line prefixed with `# ` |
| Markdown, HTML, XML | an HTML comment (`<!--` … `-->`) containing the three lines |

The year is the year in which the file was created. A script with an
interpreter line (`#!/usr/bin/env bash`) has the header directly after it.
YAML front matter precedes the header in Markdown files that need it. Headers
missing from a contribution are completed by the maintainers before merge.

Files without a comment syntax have no header and are covered by the license
of the repository: JSON (schemas, golden files), lock files, binary test data,
`LICENSE` and `NOTICE`. Texts reproduced from third parties, such as
[CODE_OF_CONDUCT.md](../../CODE_OF_CONDUCT.md) (Contributor Covenant 2.1,
CC BY 4.0), are kept verbatim with their own license statement.

The `headers` group of `tools/ci/repo-hygiene.sh` reports files in formats
with comments whose first lines lack the copyright line of the producer header
(check `producer-header`), files with per-file license identifier lines
(check `spdx-line`) and files with marks of an authoring tool (check
`tool-mark`). Exceptions are listed in `tools/ci/hygiene-allow.txt`.

## Third-party code

Code copied from another project is not accepted unless its license is in the
allowed list below, its origin and license are stated in a comment at the top
of the file, the notices required by its license are added to `NOTICE`, and
the file is listed with a reason in `tools/ci/hygiene-allow.txt`.

The crate `crates/nurt-pdf-extract` is such a copy: the source of pdf-extract
0.12.1 (MIT), built on a newer release of lopdf than the upstream release
allows. Its license text is `crates/nurt-pdf-extract/LICENSE-MIT`.

## Dependencies

All dependencies are declared in `[workspace.dependencies]` of the root
`Cargo.toml`, are taken from crates.io only, and are checked by
[cargo-deny](https://github.com/EmbarkStudios/cargo-deny) with the policy in
[deny.toml](../../deny.toml):

```sh
cargo deny --locked check
```

### Allowed licenses

Licenses that are permissive and compatible with the Apache License 2.0:

| License | SPDX identifier |
|---|---|
| MIT | `MIT` |
| Apache License 2.0, also with the LLVM exception | `Apache-2.0`, `Apache-2.0 WITH LLVM-exception` |
| BSD 2-Clause and 3-Clause | `BSD-2-Clause`, `BSD-3-Clause` |
| ISC | `ISC` |
| zlib | `Zlib` |
| The Unlicense | `Unlicense` |
| Creative Commons Zero 1.0 | `CC0-1.0` |
| Zero-Clause BSD | `0BSD` |
| Unicode License v3 (data) | `Unicode-3.0` |
| Boost Software License 1.0 | `BSL-1.0` |

An expression with alternatives (`A OR B`) is accepted when one alternative is
allowed; the permissive alternative is the one used.

### Licenses accepted for individual crates

The following licenses are accepted only for the named crates, after an
individual review; each entry in `deny.toml` states the reason.

| License | Crate | Reason |
|---|---|---|
| `CDLA-Permissive-2.0` | `webpki-roots` | root certificates as data; fallback when the system certificate store is unavailable |
| `bzip2-1.0.6` | `libbz2-rs-sys` | permissive license of the bzip2 implementation |
| `MPL-2.0` | `uluru`, `option-ext` | file-level copyleft of unmodified transitive dependencies |
| `NCSA` | `libfuzzer-sys` | fuzzing runtime, not part of distributed artifacts |

### Excluded licenses and crates

Copyleft licenses are not accepted for any dependency: GPL, LGPL and AGPL in
every version, and licenses that restrict the field of use. The product does
not embed or link code of GPL or LGPL programs (for example shells, GNU core
utilities, Git, BusyBox, GNU Readline, libsystemd).

The following crates are banned in `deny.toml`:

| Crate | Reason | Used instead |
|---|---|---|
| `sequoia-openpgp` | LGPL-2.0-or-later | GnuPG, started as a program |
| `openssl`, `openssl-sys` | system OpenSSL library | `rustls` |
| `git2`, `libgit2-sys` | libgit2 (GPL-2.0 with a linking exception) | `gix` |
| `serde_yaml` | deprecated and unmaintained | `saphyr`, `saphyr-parser` |
| `ini` | dual licensed with LGPL-3.0-or-later | `rust-ini` and an own parser |
| `libsystemd-sys` | libsystemd (LGPL-2.1) | `zbus`; native reader of journal files |
| `unrar`, `unrar_sys` | the UnRAR license forbids creating a RAR compressor | none |

### Adding a dependency

A pull request that adds a dependency states the crate, the version, the
license and the reason, and updates `Cargo.lock`. Build-time tools whose
license does not cover the produced artifacts are assessed individually.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
