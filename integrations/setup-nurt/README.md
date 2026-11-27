<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# setup-nurt

GitHub Action that installs Danaco Nurt and adds `nurt` to the `PATH`.

## Usage

```yaml
steps:
  - uses: DanacoOfficial/setup-nurt@v1
    with:
      version: '1'
  - run: nurt version
```

The action runs on Linux, macOS and Windows runners (`x64` and `arm64`).

## Inputs

| Name | Type | Default | Description |
|---|---|---|---|
| `version` | text | `latest` | `latest`, an exact SemVer version, or a version range. |
| `verify` | `true` or `false` | `true` | Verify the SHA-256 digest of the archive against the `checksums.txt` file of the release. |

## Outputs

| Name | Description |
|---|---|
| `path` | Absolute path of the installed `nurt` executable. |
| `version` | Installed version without the `v` prefix. |

## Version selection

`latest` selects the highest release without a pre-release suffix. An exact SemVer version is installed without querying the release list and may be a pre-release.

A range selects the highest matching release. Supported forms:

| Form | Example | Meaning |
|---|---|---|
| Partial version | `1`, `1.4`, `1.x`, `1.4.x` | Any release with the given leading numbers. |
| Caret | `^1.2.3` | `>=1.2.3 <2.0.0`; `^0.2.3` is `>=0.2.3 <0.3.0`. |
| Tilde | `~1.2.3` | `>=1.2.3 <1.3.0`. |
| Comparators | `>=1.2.0 <2.0.0` | All comparators must hold. |
| Alternatives | `1.x \|\| 2.x` | Any alternative may hold. |

Pre-releases match a range only through an exact version. The release list is read from the first 100 releases returned by the GitHub API.

## Release layout

For release `X.Y.Z` the action downloads, from `https://github.com/DanacoOfficial/danaco-nurt/releases/download/vX.Y.Z/`:

| File | Content |
|---|---|
| `nurt-X.Y.Z-<target>.tar.gz` | Linux and macOS archive with `nurt` at its root. |
| `nurt-X.Y.Z-<target>.zip` | Windows archive with `nurt.exe` at its root. |
| `checksums.txt` | Lines of `<digest>  <file name>` for every archive. |

Targets:

| System | `x64` | `arm64` |
|---|---|---|
| Linux | `x86_64-unknown-linux-musl` | `aarch64-unknown-linux-musl` |
| macOS | `x86_64-apple-darwin` | `aarch64-apple-darwin` |
| Windows | `x86_64-pc-windows-msvc` | `aarch64-pc-windows-msvc` |

## Tool cache

The installation is stored in `$RUNNER_TOOL_CACHE/nurt/<version>/<arch>` with a `<arch>.complete` marker, the layout used by `@actions/tool-cache`. A later run that resolves to the same version uses the cached directory without downloading.

## Environment

| Variable | Description |
|---|---|
| `NURT_DOWNLOAD_BASE` | Replaces the release download location. |
| `NURT_RELEASES_URL` | Replaces the release list location. |

## Tests

```
bash integrations/setup-nurt/tests/run.sh
```

The tests resolve versions, select artifacts and verify checksums against a release tree created in a temporary directory. They require `bash`, `curl`, `tar` and `sha256sum` or `shasum`. The workflow in `ci/setup-nurt.yml` runs them on the three operating systems.
