# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           integrations/containers/ci/Dockerfile
# Responsibility:   Container image definition of integrations/containers/ci/Dockerfile.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

FROM debian:bookworm-slim
ARG TARGETARCH
ARG VERSION=0.0.0
ARG REVISION=unknown
LABEL org.opencontainers.image.title="nurt-ci" \
      org.opencontainers.image.description="Command shell and language where every change can be undone" \
      org.opencontainers.image.source="https://github.com/DanacoOfficial/danaco-nurt" \
      org.opencontainers.image.url="https://nurt.danaco-group.pl" \
      org.opencontainers.image.documentation="https://nurt.danaco-group.pl" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.vendor="Danaco Holding Group" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 10001 --shell /usr/sbin/nologin nurt \
 && install -d -m 0700 -o nurt -g nurt /var/lib/nurt /workspace
COPY --chmod=0755 ${TARGETARCH}/nurt /usr/local/bin/nurt
COPY LICENSE NOTICE /usr/share/doc/nurt/
ENV NURT_STATE_DIR=/var/lib/nurt
USER 10001:10001
WORKDIR /workspace
CMD ["nurt"]

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
