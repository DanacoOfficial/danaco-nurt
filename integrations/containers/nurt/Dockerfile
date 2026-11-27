FROM debian:bookworm-slim AS rootfs
RUN install -d -m 0755 /rootfs/usr/local/bin /rootfs/usr/share/doc/nurt \
 && install -d -m 1777 /rootfs/tmp \
 && install -d -m 0700 -o 65532 -g 65532 /rootfs/var/lib/nurt /rootfs/workspace

FROM scratch
ARG TARGETARCH
ARG VERSION=0.0.0
ARG REVISION=unknown
LABEL org.opencontainers.image.title="nurt" \
      org.opencontainers.image.description="Command shell and language where every change can be undone" \
      org.opencontainers.image.source="https://github.com/DanacoOfficial/danaco-nurt" \
      org.opencontainers.image.url="https://nurt.danaco-group.pl" \
      org.opencontainers.image.documentation="https://nurt.danaco-group.pl" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.vendor="Danaco Holding Group" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"
COPY --from=rootfs /rootfs/ /
COPY --chmod=0755 ${TARGETARCH}/nurt /usr/local/bin/nurt
COPY LICENSE NOTICE /usr/share/doc/nurt/
ENV NURT_STATE_DIR=/var/lib/nurt
USER 65532:65532
WORKDIR /workspace
ENTRYPOINT ["/usr/local/bin/nurt"]
