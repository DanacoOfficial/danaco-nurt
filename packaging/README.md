<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Packaging

Danaco Nurt is a command shell and language where every change can be undone. This directory holds its package definitions and installers.

## Installation

| System | Command |
|---|---|
| Debian, Ubuntu | `apt install nurt` (repository setup below) |
| Fedora, RHEL | `dnf install nurt` |
| macOS, Linux | `brew install DanacoOfficial/tap/nurt` |
| Windows | `winget install Danaco.Nurt` |
| Windows | `scoop install nurt` |
| Windows | `choco install nurt` |
| Linux, macOS | `curl -fsSL https://nurt.danaco-group.pl/install.sh \| sh` |
| Windows | `irm https://nurt.danaco-group.pl/install.ps1 \| iex` |

The installed program is checked with `nurt version`; the first output line has the form `nurt <version>`.

### apt repository

Install the public signing key and the source definition, then install the package:

```sh
sudo install -d /etc/apt/keyrings
sudo curl -fsSL https://nurt.danaco-group.pl/danaco-nurt.asc -o /etc/apt/keyrings/danaco-nurt.asc
sudo curl -fsSL https://raw.githubusercontent.com/DanacoOfficial/nurt/main/packaging/apt-repo/nurt.sources \
    -o /etc/apt/sources.list.d/nurt.sources
sudo apt update
sudo apt install nurt
```

The `Signed-By` field of `nurt.sources` restricts the key to this repository.

### Installation scripts

`install.sh` installs `nurt` to `$HOME/.local/bin` without elevated privileges. It verifies the SHA-256 digest of the archive against `checksums.txt` of the release. When a public key is given and `gpg` is installed, it also verifies `checksums.txt.asc`.

| Option | Environment variable | Default |
|---|---|---|
| `--version VERSION` | `NURT_VERSION` | latest release |
| `--bin-dir DIR` | `NURT_INSTALL_DIR` | `$HOME/.local/bin` |
| `--public-key FILE\|URL` | `NURT_PUBLIC_KEY` | none |
| | `NURT_RELEASES_URL` | `https://github.com/DanacoOfficial/nurt/releases` |

`install.ps1` takes `NURT_VERSION`, `NURT_INSTALL_DIR` (default `%LOCALAPPDATA%\Programs\Nurt\bin`), `NURT_NO_PATH_UPDATE` and `NURT_RELEASES_URL` from the environment, verifies the SHA-256 digest and adds the directory to the user `PATH`.

## Release archives

The release job publishes the archives of every target and `checksums.txt` in the format of `sha256sum`. `pack.py` assembles them: the program archive `nurt-<version>-<target>` and, when `--library` is given, the library archive `nurt-lib-<version>-<target>`. `checksums.txt` lists both kinds, and `checksums.txt.asc` signs both.

| Target | Archive | Used by |
|---|---|---|
| `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` | `nurt-<version>-<target>.tar.gz`, `nurt-lib-<version>-<target>.tar.gz` | deb, rpm, Homebrew on Linux, `install.sh` when the release has no musl archive |
| `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` | `nurt-<version>-<target>.tar.gz` | `install.sh` (preferred), the container images, the GitHub Action `setup-nurt` and the dev container feature; optional, a release without them is installed from the glibc archives by `install.sh` only |
| `x86_64-apple-darwin`, `aarch64-apple-darwin` | `nurt-<version>-<target>.tar.gz`, `nurt-lib-<version>-<target>.tar.gz` | `install.sh`, Homebrew |
| `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc` | `nurt-<version>-<target>.zip`, `nurt-lib-<version>-<target>.zip` | winget, Scoop, Chocolatey, `install.ps1` |
| source | `nurt-<version>-source.tar.gz` | Homebrew core formula |

The placeholder `@SHA256:lib-<target>@` of `render.py` reads the digest of the library archive of a target.

The program archive of Linux and macOS contains the directory `nurt-<version>-<target>/` with:

| Path | Content | Required |
|---|---|---|
| `nurt` | executable | yes |
| `LICENSE`, `NOTICE`, `README.md`, `CHANGELOG.md` | texts | yes |
| `man/man1/*.1` | manual pages | no |
| `completions/nurt.bash`, `completions/_nurt`, `completions/nurt.fish` | shell completions | no |
| `doc/` | the documentation site | no |
| `service/` | unit files of the service mode | no |

The library archive contains the directory `nurt-lib-<version>-<target>/` with:

| Path | Content | Required |
|---|---|---|
| `libnurt.so.1` | shared library (glibc Linux); `libnurt.1.dylib` on macOS | yes |
| `nurt.h`, `nurt.pc` | C header and pkg-config file; `nurt.pc` defines `prefix`, `libdir` and `includedir` on separate lines | yes |
| `LICENSE`, `NOTICE` | license texts | yes |

The library is not packed for musl targets.

The musl archive is built with the target `x86_64-unknown-linux-musl` (`rustup target add x86_64-unknown-linux-musl`) and `zig cc` as the C compiler of the target. The job `musl` of the workflow `release.yml` (the copy `packaging/ci/release.yml`) does exactly this and uploads the archive as the artifact `musl-archive`. The C compiler and the linker are set apart: the compiler is the script `packaging/release/zig-cc-musl.sh`, which runs `zig cc -target x86_64-linux-musl` (the executable of zig is taken from `ZIG`, `zig` by default) and drops the option `--target=...` that the `cc` crate adds; the linker is the default `cc`:

```sh
rustup target add x86_64-unknown-linux-musl
export CC_x86_64_unknown_linux_musl="$PWD/packaging/release/zig-cc-musl.sh"
export CFLAGS_x86_64_unknown_linux_musl="-fno-sanitize=undefined"
cargo build --release --target x86_64-unknown-linux-musl -p nurt-cli
python3 -I packaging/release/pack.py --root . --version <version> --target x86_64-unknown-linux-musl \
    --binary target/x86_64-unknown-linux-musl/release/nurt --output archives --completions <dir> --site <dir>
```

The linker stays the default `cc`: `zig cc` as the linker adds a second C runtime start file to the one of the Rust target and the link fails with a duplicate `_start`. `-fno-sanitize=undefined` is needed because `zig cc` enables the undefined behavior sanitizer by default and `aws-lc-sys` then leaves `__ubsan_handle_type_mismatch_v1` undefined. The result is a static-pie executable with no `NEEDED` entries; `release.yml` checks it with `check-dependencies.sh`.

`build-deb.sh` takes the library archive with `--lib-artifact`; `nurt.spec.in` lists it as `Source1`.

Windows archives hold their files at the top level: the program archive `nurt.exe`, `LICENSE`, `NOTICE`, `README.md`, `CHANGELOG.md`, `completions/` and `doc/`; the library archive `nurt.dll`, `nurt.h`, `LICENSE` and `NOTICE`. The packages only deliver the service files; `nurt service install` installs the service.

## Dependency check

`tools/ci/check-dependencies.sh [--format elf|pe] FILE...` reads the dynamic section of every file (`readelf -d`, entries `NEEDED`) and fails when a file names a shared library outside the permitted list: `libc.so.6`, `libm.so.6`, `libgcc_s.so.1`, `ld-linux-x86-64.so.2`, `ld-linux-aarch64.so.1` and `linux-vdso.so.1`. A statically linked file passes. The format is taken from the first bytes of the file when `--format` is omitted. The environment variable `READELF` replaces the `readelf` program. The workflow `release.yml` runs the script on `nurt` and `libnurt.so.1` of both glibc archives before it signs the checksums. The check of Windows files (`check_pe`, list `WINDOWS_ALLOWED_DLLS`) is reserved in the script and not implemented. The tests are in `tests/test_check_dependencies.py`.

## Directories

| Directory | Content |
|---|---|
| `release/` | `render.py`, the generator of versions and digests |
| `deb/` | `build-deb.sh`, DEP-5 `copyright` |
| `rpm/` | `nurt.spec.in` for `nurt`, `libnurt`, `libnurt-devel` |
| `apt-repo/` | `mkrepo.py`, `nurt.sources` |
| `homebrew/` | `nurt.rb.in` (release archives), `core/nurt.rb.in` (source build) |
| `winget/` | manifests, schema version 1.6.0 |
| `scoop/` | `nurt.json.in` with `checkver` and `autoupdate` |
| `chocolatey/` | `nurt.nuspec.in`, `tools/chocolateyinstall.ps1.in` |
| `install/` | `install.sh`, `install.ps1` |
| `tests/` | tests and the container installation check |
| `ci/` | workflows for packaging checks and publication |

## Generating manifests

Files with the suffix `.in` are templates. `render.py` writes them without the suffix to an output directory, using the digests of `checksums.txt`.

```sh
python3 packaging/release/render.py --checksums checksums.txt --output rendered [--version VERSION] [--epoch SECONDS]
```

The version is taken from the archive names when `--version` is omitted. The date placeholders use `--epoch` or `SOURCE_DATE_EPOCH`. The command fails when a placeholder cannot be resolved.

| Placeholder | Value |
|---|---|
| `@VERSION@` | release version |
| `@DEB_VERSION@`, `@RPM_VERSION@`, `@RPM_RELEASE@`, `@CHOCO_VERSION@` | version in the notation of the package format |
| `@DATE@`, `@RPM_DATE@` | release date |
| `@SHA256:<target>@` | digest of the archive of the target; `source` names the source archive |

## Building packages

```sh
SOURCE_DATE_EPOCH=<seconds> sh packaging/deb/build-deb.sh \
    --artifact nurt-<version>-x86_64-unknown-linux-gnu.tar.gz \
    --version <version> --arch amd64 --output debs
```

The script produces `nurt`, `libnurt1` and `libnurt-dev`. The same inputs and `SOURCE_DATE_EPOCH` give identical files. RPM packages are built from the rendered `rpm/nurt.spec` with `rpmbuild -bb`.

The apt repository is generated by `apt-repo/mkrepo.py`:

```sh
python3 packaging/apt-repo/mkrepo.py --packages debs --output apt --suite stable --sign-key <key>
```

The key is a key identifier known to the `gpg` keyring at release time. Without `--sign-key` the repository is not signed.

## Tests

```sh
python3 -I -m unittest discover -s packaging/tests -p 'test_*.py'
```

The tests build the packages and run the scripts against a mock release archive. The checks that require `shellcheck`, `gpg` or `dpkg-deb` are skipped when the program is missing. `tests/container-install.sh` installs the packages in a clean container; the workflow `ci/packaging.yml` runs it on Debian, Ubuntu and Fedora.

## Publication workflow

The workflows in `ci/` are installed as `.github/workflows/packaging.yml` and `.github/workflows/release.yml`. `release.yml` runs in the environment `release` and reads these secrets:

| Secret | Use |
|---|---|
| `RELEASE_SIGNING_KEY` | private key that signs `checksums.txt` and the apt repository |
| `APT_DEPLOY_KEY`, `APT_DEPLOY_TARGET`, `APT_DEPLOY_KNOWN_HOSTS` | upload of the apt repository |
| `TAP_PUSH_TOKEN` | push access to `DanacoOfficial/homebrew-tap` and `DanacoOfficial/scoop-bucket` |
| `WINGET_PUBLISH_TOKEN` | pull requests to the winget package repository |
| `CHOCOLATEY_API_KEY` | push to the Chocolatey community repository |
