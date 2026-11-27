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
