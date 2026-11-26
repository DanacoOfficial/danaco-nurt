#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Clean-system check of a Linux release archive.
#
# Runs the archive in a container of a minimal image and checks that the
# executable works without anything but the system of the image: it unpacks
# the archive, runs `nurt version` and `nurt doctor`, changes a file, undoes
# the change and checks the state after the undo.
#
# Usage:
#
#   tools/release/clean-linux-check.sh <archive.tar.gz>
#
# The archive nurt-<version>-x86_64-unknown-linux-gnu.tar.gz is checked in
# debian:stable-slim. The archive nurt-<version>-x86_64-unknown-linux-musl.tar.gz
# is checked in alpine:latest. When the glibc archive is given and the musl
# archive of the same version lies in the same directory, both are checked.
#
# Environment (all optional):
#
#   NURT_CONTAINER_ENGINE   docker or podman; the first one that answers `info`
#   NURT_DEBIAN_IMAGE       image for the glibc archive (debian:stable-slim)
#   NURT_ALPINE_IMAGE       image for the musl archive (alpine:latest)
#
# Exit code 0 means that every checked archive passed.

set -euo pipefail

usage() {
  sed -n '/^# Usage:/,/^# Exit code/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

log() { printf '[clean-linux-check] %s\n' "$*" >&2; }
die() { printf '[clean-linux-check] error: %s\n' "$*" >&2; exit 1; }

[ $# -eq 1 ] || usage
case "$1" in -h | --help) usage ;; esac
archive="$1"
[ -f "$archive" ] || die "no such archive: $archive"
archive="$(cd "$(dirname "$archive")" && pwd)/$(basename "$archive")"

engine="${NURT_CONTAINER_ENGINE:-}"
if [ -z "$engine" ]; then
  for candidate in docker podman; do
    if command -v "$candidate" > /dev/null 2>&1 && timeout 30 "$candidate" info > /dev/null 2>&1; then
      engine="$candidate"
      break
    fi
  done
  [ -n "$engine" ] || die "neither docker nor podman is usable"
fi
debian_image="${NURT_DEBIAN_IMAGE:-docker.io/library/debian:stable-slim}"
alpine_image="${NURT_ALPINE_IMAGE:-docker.io/library/alpine:latest}"

# The script that runs inside the container. It receives the file name of the
# archive and exits with 0 only when every check passed.
inside='
set -eu
archive="/archive/$1"
mkdir -p /opt/release /work "$HOME"
tar -xzf "$archive" -C /opt/release
nurt=$(ls -d /opt/release/nurt-*/nurt)
[ -x "$nurt" ] || { echo "no executable in the archive"; exit 1; }
echo "--- nurt version"
"$nurt" version
echo "--- nurt doctor"
"$nurt" doctor
echo "--- change and undo"
printf "before\n" > /work/file.txt
"$nurt" "write \"/work/file.txt\" text: \"after\" mode: overwrite"
[ "$(cat /work/file.txt)" = "after" ] || { echo "the change was not applied"; exit 1; }
"$nurt" undo
[ "$(cat /work/file.txt)" = "before" ] || { echo "undo did not restore the file"; exit 1; }
[ "$(ls /work)" = "file.txt" ] || { echo "unexpected files after undo: $(ls /work)"; exit 1; }
echo "--- passed"
'

check() {
  local file="$1" image="$2"
  log "$(basename "$file") in $image"
  "$engine" run --rm --pull=missing \
    --network=none -e HOME=/root \
    -v "$(dirname "$file"):/archive:ro" \
    "$image" sh -c "$inside" sh "$(basename "$file")"
}

failed=0
case "$archive" in
  *-linux-musl.tar.gz) check "$archive" "$alpine_image" || failed=1 ;;
  *-linux-gnu.tar.gz)
    check "$archive" "$debian_image" || failed=1
    musl="${archive%-linux-gnu.tar.gz}-linux-musl.tar.gz"
    if [ -f "$musl" ]; then check "$musl" "$alpine_image" || failed=1; fi
    ;;
  *) die "the archive name must end in -linux-gnu.tar.gz or -linux-musl.tar.gz" ;;
esac

if [ "$failed" -ne 0 ]; then
  log "FAILED"
  exit 1
fi
log "passed"
