<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           integrations/devcontainer/README.md -->
<!-- Responsibility:   Documents Danaco Nurt devcontainer Feature. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Danaco Nurt devcontainer Feature

Development container Feature that installs Danaco Nurt, a command shell and language where every change can be undone, and recommends the `danaco.nurt` VS Code extension.

## Usage

```json
{
  "image": "mcr.microsoft.com/devcontainers/base:bookworm",
  "features": {
    "ghcr.io/danacoofficial/features/nurt:1": {
      "version": "latest"
    }
  }
}
```

## Options

| Option | Type | Default | Description |
|---|---|---|---|
| `version` | string | `latest` | `latest` selects the highest stable release. An exact version such as `1.4.2` is installed as given. A major or minor prefix such as `1` or `1.4` selects the highest stable release with that prefix. |

## Installation

`install.sh` selects the Linux archive for the architecture (`x86_64` or `aarch64`, statically linked against musl), downloads it with `checksums.txt` from the GitHub release of `DanacoOfficial/danaco-nurt`, verifies the SHA-256 digest and installs `nurt` into `/usr/local/bin`. The script runs on Debian, Ubuntu, Alpine and Red Hat based images; it installs `curl`, `tar` and `ca-certificates` with the package manager of the image when they are missing.

Environment variables for testing against a different release location:

| Variable | Description |
|---|---|
| `NURT_DOWNLOAD_BASE` | Replaces the release download location. |
| `NURT_RELEASES_URL` | Replaces the release list location. |
| `NURT_INSTALL_DIR` | Replaces the install directory. |

## Tests

| Command | Requirement | Check |
|---|---|---|
| `bash test/install-offline.sh` | `bash`, `curl`, `tar`, `sha256sum` | Version selection, checksum verification and installation against a release tree in a temporary directory. |
| `devcontainer features test --project-folder . --features nurt` | `@devcontainers/cli`, Docker, network | Scenarios in `test/nurt`: default options, major prefix, Alpine image. |

The workflow in `ci/devcontainer.yml` runs both and publishes the Feature to `ghcr.io/danacoofficial/features/nurt` when a release is published.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
