<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           integrations/containers/README.md -->
<!-- Responsibility:   Documents Container images. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Container images

Image definitions for Danaco Nurt. Images contain a prebuilt `nurt` executable copied from the build context; nothing is compiled in the image.

| Image | Base | Content | User |
|---|---|---|---|
| `ghcr.io/danacoofficial/nurt` | `scratch` | `nurt`, `LICENSE`, `NOTICE` | `65532:65532` |
| `ghcr.io/danacoofficial/nurt-ci` | `debian:bookworm-slim` | `nurt`, `git`, `ca-certificates`, `LICENSE`, `NOTICE` | `nurt` (`10001:10001`) |
| `ghcr.io/danacoofficial/nurt-devcontainer` | `mcr.microsoft.com/devcontainers/base:bookworm` | `nurt`, `git`, `ca-certificates`, `LICENSE`, `NOTICE`, recommendation of the `danaco.nurt` extension | `vscode` |

All images are built for `linux/amd64` and `linux/arm64` and carry the OCI labels `org.opencontainers.image.source`, `licenses` (`Apache-2.0`) and `vendor` (`Danaco Holding Group`), as well as `title`, `description`, `url`, `documentation`, `version` and `revision`.

## Usage

```
docker run --rm -v "$PWD:/workspace" ghcr.io/danacoofficial/nurt version
docker run --rm -v "$PWD:/workspace" ghcr.io/danacoofficial/nurt-ci nurt version
```

The `nurt` image has `nurt` as entry point. The `nurt-ci` image has no entry point and starts `nurt` by default, so a CI system can run any command in it. The `nurt` image contains no CA certificate bundle and no `git`; use `nurt-ci` for jobs that need either.

## State directory

`NURT_STATE_DIR` is `/var/lib/nurt` in every image. The directory belongs to the image user and is on the container file system layer (`overlay`); the journal operates on it without a volume. Mount a volume at `/var/lib/nurt` to keep the state across containers. The working directory is `/workspace`.

## Build context

The build context has the following layout:

```
amd64/nurt
arm64/nurt
LICENSE
NOTICE
```

Build one image:

```
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  --file integrations/containers/ci/Dockerfile \
  --build-arg VERSION=1.0.0 \
  --build-arg REVISION=<commit> \
  <context>
```

`ci/containers.yml` stages the context from the Linux archives of a release after verifying `checksums.txt`, builds the three images, publishes them with an SBOM and provenance attestation, signs them with `cosign` (keyless) and runs `tests/smoke.sh`.

## Tests

| Command | Requirement | Check |
|---|---|---|
| `bash tests/check-dockerfiles.sh` | `bash`, `grep` | Labels, license files, numeric non-root users, no compilation steps, no floating tags. |
| `hadolint --config .hadolint.yaml */Dockerfile` | `hadolint` | Dockerfile rules. |
| `bash tests/smoke.sh IMAGE [nurt\|ci\|devcontainer]` | `docker` | Version output, state directory, user, labels, license files. |

## Publication

Publishing requires write access to the `ghcr.io/danacoofficial` namespace through the workflow token and an OIDC identity for `cosign`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
