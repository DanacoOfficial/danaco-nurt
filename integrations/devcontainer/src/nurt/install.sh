#!/bin/sh
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Installs Danaco Nurt into a development container.
#
# Option (environment): VERSION is "latest", an exact SemVer version, or a
# major or major.minor prefix such as "1" or "1.4".
# NURT_DOWNLOAD_BASE, NURT_RELEASES_URL and NURT_INSTALL_DIR override the
# release locations and the install directory.

set -eu

VERSION=${VERSION:-latest}
DOWNLOAD_BASE=${NURT_DOWNLOAD_BASE:-https://github.com/DanacoOfficial/nurt/releases/download}
RELEASES_URL=${NURT_RELEASES_URL:-https://api.github.com/repos/DanacoOfficial/nurt/releases?per_page=100}
INSTALL_DIR=${NURT_INSTALL_DIR:-/usr/local/bin}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

have() {
  command -v "$1" >/dev/null 2>&1
}

install_packages() {
  if have apt-get; then
    apt-get update -y
    apt-get install -y --no-install-recommends "$@"
    rm -rf /var/lib/apt/lists/*
  elif have apk; then
    apk add --no-cache "$@"
  elif have dnf; then
    dnf install -y "$@"
  elif have microdnf; then
    microdnf install -y "$@"
  elif have yum; then
    yum install -y "$@"
  else
    fail "no supported package manager; install curl, tar and ca-certificates first"
  fi
}

ensure_tools() {
  if have curl && have tar && have sha256sum; then
    return 0
  fi
  install_packages curl tar ca-certificates
}

release_target() {
  case $(uname -m) in
    x86_64 | amd64) echo x86_64-unknown-linux-musl ;;
    aarch64 | arm64) echo aarch64-unknown-linux-musl ;;
    *) fail "unsupported architecture '$(uname -m)'" ;;
  esac
}

# Prints the highest stable version that starts with the prefix $1 (empty for any).
resolve_from_listing() {
  prefix=$1
  listing=$(mktemp)
  curl --fail --silent --show-error --location --retry 3 --output "$listing" "$RELEASES_URL" ||
    fail "cannot download the release listing; pass an exact version"
  grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' "$listing" |
    sed -E 's/.*"v?([^"]*)"$/\1/' |
    grep -E "^${prefix}[0-9]+(\.[0-9]+)*$" |
    grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' |
    sort -t. -k1,1n -k2,2n -k3,3n |
    tail -n 1
  rm -f "$listing"
}

resolve_version() {
  case $VERSION in
    latest)
      resolved=$(resolve_from_listing '')
      ;;
    v[0-9]*.[0-9]*.[0-9]* | [0-9]*.[0-9]*.[0-9]*)
      resolved=${VERSION#v}
      ;;
    [0-9]* | v[0-9]*)
      case ${VERSION#v} in
        *[!0-9.]* | *..* | .* | *.) fail "invalid version '$VERSION'" ;;
      esac
      resolved=$(resolve_from_listing "$(printf '%s' "${VERSION#v}" | sed 's/\./\\./g')\\.")
      ;;
    *)
      fail "invalid version '$VERSION'; use latest, an exact version or a major or minor prefix"
      ;;
  esac
  [ -n "$resolved" ] || fail "no release matches version '$VERSION'"
  printf '%s\n' "$resolved"
}

main() {
  ensure_tools
  version=$(resolve_version)
  target=$(release_target)
  archive="nurt-$version-$target.tar.gz"
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT

  curl --fail --silent --show-error --location --retry 3 \
    --output "$work/$archive" "$DOWNLOAD_BASE/v$version/$archive" ||
    fail "cannot download $archive"
  curl --fail --silent --show-error --location --retry 3 \
    --output "$work/SHA256SUMS" "$DOWNLOAD_BASE/v$version/SHA256SUMS" ||
    fail "cannot download SHA256SUMS for version $version"

  expected=$(awk -v n="$archive" '{ f = $2; sub(/^\*/, "", f); if (f == n) { print tolower($1); exit } }' "$work/SHA256SUMS")
  [ -n "$expected" ] || fail "no checksum listed for $archive"
  actual=$(sha256sum "$work/$archive" | awk '{ print tolower($1) }')
  [ "$expected" = "$actual" ] ||
    fail "checksum mismatch for $archive: expected $expected, computed $actual"

  mkdir -p "$work/extract" "$INSTALL_DIR"
  tar -xzf "$work/$archive" -C "$work/extract"
  [ -f "$work/extract/nurt" ] || fail "$archive does not contain nurt"
  install -m 0755 "$work/extract/nurt" "$INSTALL_DIR/nurt"
  printf 'installed Danaco Nurt %s in %s\n' "$version" "$INSTALL_DIR"
}

main
