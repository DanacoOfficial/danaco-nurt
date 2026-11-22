---
title: Installation
order: 2
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Installation

Danaco Nurt is one executable for each operating system: `nurt` on Linux and macOS, `nurt.exe` on Windows. The executable has no runtime dependencies.

## Supported systems

| System | Architectures |
|---|---|
| Linux | x86_64, aarch64 |
| macOS | x86_64, aarch64 |
| Windows | x86_64, aarch64 |

## Release artifacts

| Channel | Identifier |
|---|---|
| GitHub release | `github.com/DanacoOfficial/nurt` |
| Container image | `ghcr.io/danacoofficial/nurt` |
| winget | `Danaco.Nurt` |
| NuGet (library binding) | `Danaco.Nurt` |
| Maven (library binding) | `pl.danaco-group:nurt` |

Every release publishes SHA-256 sums of its artifacts. The installers `install.sh` and `install.ps1` verify the sum before they place the executable.

## Build from source

The build needs a Rust toolchain installed with rustup and a C compiler. The toolchain version is pinned in `rust-toolchain.toml`.

```text
git clone https://github.com/DanacoOfficial/nurt.git
cd nurt
cargo build --release --locked
```

The executable is written to `target/release/nurt` (`target\release\nurt.exe` on Windows).

## Verification

```text
nurt --version
```

The command prints the version, the copyright line and the license; the format is described in [Command line](command-line.md). `nurt doctor` reports the properties of the file system, the permissions, the service and the language servers that it found.

## License

The product is licensed under the Apache License, Version 2.0. Distributions include the `NOTICE` file. See [Compatibility](compatibility.md) for the use of the name.
