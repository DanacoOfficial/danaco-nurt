---
title: Installation
order: 2
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/installation.md -->
<!-- Responsibility:   Documents Installation. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Installation

Danaco Nurt is one executable for each operating system: `nurt` on Linux and macOS, `nurt.exe` on Windows. The musl build for Linux and the Windows build have no runtime dependencies. The glibc build for Linux links the system libraries `libc.so.6`, `libm.so.6` and `libgcc_s.so.1` dynamically.

## Systems and archives

The code is built and tested on the systems of the first table. The second table lists the release archives of release 0.1.2.

| System | Architectures of the code | Release archive | Tests |
|---|---|---|---|
| Linux | x86_64, aarch64 | glibc and musl, each x86_64 and aarch64 | continuous integration |
| Windows | x86_64, aarch64 | MSVC, x86_64 and aarch64 | continuous integration |
| macOS | x86_64, aarch64 | none; build from source | continuous integration on macOS runners; no archive is published for release 0.1.2, see "Known issues" of release 0.1.2 in `CHANGELOG.md` |

| Target of the archive | System |
|---|---|
| `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` | Linux with glibc |
| `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` | Linux, static executable |
| `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc` | Windows 10 or later |

The setup program for Windows requires Windows 10 version 1809 (build 17763) or later and checks it after it starts. `nurt.exe` is built with a Rust toolchain whose Windows targets support Windows 10 and later.

## Release artifacts

| Channel | Identifier | State |
|---|---|---|
| GitHub release | `github.com/DanacoOfficial/danaco-nurt` | archives and `checksums.txt` |
| Container image | `ghcr.io/danacoofficial/nurt` | published by the workflow `integrations-containers.yml` |
| winget | `Danaco.Nurt` | submitted by the workflow `release.yml` |
| NuGet (library binding) | `Danaco.Nurt` | planned; the repository has no publishing step |
| Maven (library binding) | `pl.danaco-group:nurt` | planned; the repository has no publishing step |

Every release publishes SHA-256 sums of its artifacts. The installers `install.sh` and `install.ps1` verify the sum before they place the executable.

## Install on Linux and macOS

```text
curl -fsSL https://nurt.danaco-group.pl/install.sh | sh
```

The script installs `nurt` into `$HOME/.local/bin` and needs no privileges. On Linux it takes the static musl archive when the release has one and the glibc archive otherwise. For a system-wide installation run the script as root with `--bin-dir /usr/local/bin`. The option `--public-key` verifies the signature of `checksums.txt`; the key and the procedure are in `SECURITY.md`.

## Install on Windows

```text
irm https://nurt.danaco-group.pl/install.ps1 | iex
```

By default the script installs for the current user. `nurt.exe` goes to `%LOCALAPPDATA%\Programs\Nurt\bin`, the documentation to `%LOCALAPPDATA%\Programs\Nurt\doc`, and the directory of the executable is added to the `PATH` of the user. No administrator privileges are needed.

To install for all users, start PowerShell with "Run as administrator" and set the scope before the call:

```text
$env:NURT_SCOPE = 'machine'
irm https://nurt.danaco-group.pl/install.ps1 | iex
```

The scope `machine` installs into `%ProgramFiles%\Nurt` and changes the `PATH` of the machine. The variable `NURT_INSTALL_DIR` selects another directory for the executable.

### Windows setup

Every release carries a setup wizard for each Windows architecture, `nurt-<version>-<target>-setup.exe` with the target `x86_64-pc-windows-msvc` or `aarch64-pc-windows-msvc`. The file contains the program archive of the same release, so it needs no network. The wizard runs on Windows 10 version 1809 or later and on Windows 11; its interface is in English and it needs no web runtime.

1. Download the file for the architecture of the computer and check its sum against `checksums.txt` of the release.
2. Start the file. A short animation plays; a key or a click skips it.
3. Welcome: the wizard checks the Windows version, the architecture and the free space.
4. Licence: read `LICENSE` and `NOTICE` to the end; Next is enabled at the end of the text.
5. Location: choose "Only for me" (`%LOCALAPPDATA%\Programs\Nurt`, no administrator rights) or "For all users of this PC" (`%ProgramFiles%\Nurt`; Windows asks for administrator approval when the installation starts), and the folder of `nurt.exe`.
6. Options: add the folder to `PATH`, install the documentation, create Start menu, desktop and Quick Launch shortcuts, add a Windows Terminal profile (optionally as the default profile), add tab completion to PowerShell, and write a first `nurt.toml`.
7. Review, then Install: the wizard verifies the SHA-256 sum of the embedded archive, copies the files and makes the changes. An error or Cancel undoes every change made so far.
8. Finish: choose whether to open a terminal and start `nurt` and whether to show the documentation, then close the wizard.

Every change is recorded in the journal `setup-journal.jsonl` in the installation folder (format `nurt-setup-journal/1`). The program is listed in "Installed apps"; its entry, the shortcut "Uninstall Danaco Nurt" and `uninstall.exe` in the installation folder reverse the journal and remove the program, its shortcuts, its `PATH` entry and its Windows Terminal profile. `nurt.toml` stays unless `--remove-config` is given. Starting the setup again on an installed computer offers Repair (or Update, when the setup carries another version) and Undo.

The silent mode installs without a window:

```text
nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe /S
nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe /S --scope machine --terminal-profile --completions
nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe /S --dir=D:\Tools\Nurt\bin --no-path
nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe /S --undo
```

| Switch | Effect |
|---|---|
| `/S`, `--silent` | No window; the exit code reports the result. |
| `--scope user\|machine` | For the current user (default) or for all users. |
| `--dir <path>`, `/D=<path>` | Folder of `nurt.exe`. |
| `--no-path`, `--no-docs`, `--no-start-menu`, `--no-config` | Leave out `PATH`, the documentation, the Start menu shortcuts or `nurt.toml`. |
| `--desktop`, `--quick-launch` | Create the desktop or the Quick Launch shortcut. |
| `--terminal-profile`, `--terminal-default`, `--completions` | Add the Windows Terminal profile, make it the default profile, add tab completion to PowerShell. |
| `--version <X.Y.Z>` | Fail unless the setup carries this version. |
| `--repair`, `--undo`, `--remove-config` | Install again over an installation, reverse the journal, also remove `nurt.toml`. |
| `--no-animation`, `--log <file>`, `-h` | Open the window without the animation, name the log file, print the usage. |

Exit codes: 0 done, 1 failed and rolled back, 2 invalid arguments, 3 requirements not met, 4 administrator approval declined.

The release signs `nurt.exe` and the setup files with Authenticode. The certificate is self-signed (`packaging/release/danaco-nurt-codesign.pem` in the repository), so Windows reports an unknown publisher until the certificate is trusted. `Get-AuthenticodeSignature` shows the signer `CN=Danaco Nurt`. The setup cannot pin `nurt` to the taskbar, because Windows has no supported interface for it; pin the Start menu shortcut by hand. The full description of the files, the journal and the build is in `packaging/windows-installer/README.md` of the repository.

### Administrator rights

`nurt.exe` does not ask for elevation. Its manifest declares the execution level `asInvoker`, so the program runs with the rights of the terminal that starts it. Operations that need administrator rights, such as `nurt service install`, changes in the machine part of the registry and writes to system directories, work in a terminal that runs as administrator. The manifest also declares long path support and UTF-8 as the code page of the process.

## Contents of the archives

| Archive | Files |
|---|---|
| Program, Linux and macOS, directory `nurt-<version>-<target>/` | `nurt`, `LICENSE`, `NOTICE`, `README.md`, `CHANGELOG.md`, `man/`, `completions/`, `doc/` |
| Program, Windows, files at the top level | `nurt.exe`, `LICENSE`, `NOTICE`, `README.md`, `CHANGELOG.md`, `completions/`, `doc/` |
| Setup wizard, Windows, one file `nurt-<version>-<target>-setup.exe` | the Windows program archive and the wizard that installs it |
| C library, Linux and macOS, directory `nurt-lib-<version>-<target>/` | `libnurt.so.1` or `libnurt.1.dylib`, `nurt.h`, `nurt.pc`, `LICENSE`, `NOTICE` |
| C library, Linux musl, directory `nurt-lib-<version>-<target>/` | `libnurt.a` (static), `nurt.h`, `nurt.pc`, `LICENSE`, `NOTICE` |
| C library, Windows, files at the top level | `nurt.dll`, `nurt.h`, `LICENSE`, `NOTICE` |

The C library archives are published for the targets that have a library; the installers `install.sh` and `install.ps1` install the program archive only.

The directory `doc/` holds this documentation as a site that works offline; open `doc/index.html`. The directory `completions/` holds the completion scripts for Bash, Zsh, Fish, PowerShell and Elvish.

## Build from source

The build needs a Rust toolchain installed with rustup and a C compiler. The toolchain version is pinned in `rust-toolchain.toml`.

```text
git clone https://github.com/DanacoOfficial/danaco-nurt.git
cd danaco-nurt
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

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
