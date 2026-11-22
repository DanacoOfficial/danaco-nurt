FROM mcr.microsoft.com/devcontainers/base:bookworm
ARG TARGETARCH
ARG VERSION=0.0.0
ARG REVISION=unknown
LABEL org.opencontainers.image.title="nurt-devcontainer" \
      org.opencontainers.image.description="Development container with Danaco Nurt and the Danaco Nurt VS Code extension" \
      org.opencontainers.image.source="https://github.com/DanacoOfficial/nurt" \
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
