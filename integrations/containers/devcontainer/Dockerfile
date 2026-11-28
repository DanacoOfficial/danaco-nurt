# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           integrations/containers/devcontainer/Dockerfile
# Responsibility:   Container image definition of integrations/containers/devcontainer/Dockerfile.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

FROM mcr.microsoft.com/devcontainers/base:bookworm
ARG TARGETARCH
ARG VERSION=0.0.0
ARG REVISION=unknown
LABEL org.opencontainers.image.title="nurt-devcontainer" \
      org.opencontainers.image.description="Command shell and language where every change can be undone" \
      org.opencontainers.image.source="https://github.com/DanacoOfficial/danaco-nurt" \
      org.opencontainers.image.url="https://nurt.danaco-group.pl" \
      org.opencontainers.image.documentation="https://nurt.danaco-group.pl" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.vendor="Danaco Holding Group" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      devcontainer.metadata='[{"customizations":{"vscode":{"extensions":["danaco.nurt"],"settings":{"nurt.path":"/usr/local/bin/nurt"}}}}]'
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git \
 && rm -rf /var/lib/apt/lists/* \
 && install -d -m 0755 -o vscode -g vscode /var/lib/nurt
COPY --chmod=0755 ${TARGETARCH}/nurt /usr/local/bin/nurt
COPY LICENSE NOTICE /usr/share/doc/nurt/
ENV NURT_STATE_DIR=/var/lib/nurt

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
