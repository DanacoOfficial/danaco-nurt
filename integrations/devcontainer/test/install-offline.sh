#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Tests install.sh against a release tree in a temporary directory, addressed
# through file:// URLs, with the install directory redirected to the same tree.

set -uo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
install_script="$here/../src/nurt/install.sh"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

failures=0
status=0

host_target() {
  case ${1:-$(uname -m)} in
    x86_64 | amd64) echo x86_64-unknown-linux-musl ;;
    aarch64 | arm64) echo aarch64-unknown-linux-musl ;;
    *)
      printf 'unsupported architecture: %s\n' "${1:-$(uname -m)}" >&2
      return 1
      ;;
  esac
}

target=$(host_target) || exit 1

expect() {
  local description=$1
  shift
  if "$@"; then
    printf 'ok   %s\n' "$description"
  else
    printf 'FAIL %s\n' "$description"
    failures=$((failures + 1))
  fi
}

prints() {
  [[ $("$1") == "$2" ]]
}

succeeded() {
  ((status == 0))
}

failed_with() {
  ((status != 0)) && grep -q "$1" "$work/stderr"
}

missing() {
  [[ ! -e $1 ]]
}

refused_architecture() {
  ! host_target "$1" >/dev/null 2>&1
}

prints_target() {
  [[ $(host_target amd64) == x86_64-unknown-linux-musl && $(host_target arm64) == aarch64-unknown-linux-musl ]]
}

make_release() {
  local version=$1 dir="$work/release/v$1" stage
  stage=$(mktemp -d "$work/stage.XXXXXX")
  mkdir -p "$dir"
  printf '#!/bin/sh\necho "Danaco Nurt %s"\n' "$version" >"$stage/nurt"
  chmod +x "$stage/nurt"
  tar -czf "$dir/nurt-$version-$target.tar.gz" -C "$stage" nurt
  (cd "$dir" && sha256sum "nurt-$version-$target.tar.gz" >checksums.txt)
}

run_install() {
  env VERSION="$1" NURT_INSTALL_DIR="$2" \
    NURT_DOWNLOAD_BASE="file://$work/release" NURT_RELEASES_URL="file://$work/releases.json" \
    sh "$install_script" >"$work/stdout" 2>"$work/stderr"
  status=$?
}

make_release 1.2.3
make_release 1.4.0
make_release 1.10.0
make_release 2.0.0
cat >"$work/releases.json" <<'JSON'
[
  {"tag_name": "v2.1.0-rc.1"},
  {"tag_name": "v2.0.0"},
  {"tag_name": "v1.10.0"},
  {"tag_name": "v1.4.0"},
  {"tag_name": "v1.2.3"}
]
JSON

run_install 1.2.3 "$work/bin-exact"
expect 'installs an exact version' succeeded
expect 'installed executable runs' prints "$work/bin-exact/nurt" 'Danaco Nurt 1.2.3'

run_install latest "$work/bin-latest"
expect 'latest selects the highest stable version' prints "$work/bin-latest/nurt" 'Danaco Nurt 2.0.0'

run_install 1 "$work/bin-major"
expect 'major prefix selects the highest matching version' prints "$work/bin-major/nurt" 'Danaco Nurt 1.10.0'

run_install 1.4 "$work/bin-minor"
expect 'minor prefix selects the highest matching version' prints "$work/bin-minor/nurt" 'Danaco Nurt 1.4.0'

run_install 3 "$work/bin-none"
expect 'fails when no release matches' failed_with 'no release matches version'

run_install bogus "$work/bin-bogus"
expect 'rejects an invalid version' failed_with 'invalid version'

run_install 1.2.3/../../x "$work/bin-traversal"
expect 'rejects a version with path separators' failed_with 'invalid version'

nested=$(mktemp -d "$work/stage.XXXXXX")
mkdir -p "$nested/nurt-3.0.0-$target"
printf '#!/bin/sh\necho "Danaco Nurt 3.0.0"\n' >"$nested/nurt-3.0.0-$target/nurt"
chmod +x "$nested/nurt-3.0.0-$target/nurt"
mkdir -p "$work/release/v3.0.0"
tar -czf "$work/release/v3.0.0/nurt-3.0.0-$target.tar.gz" -C "$nested" "nurt-3.0.0-$target"
(cd "$work/release/v3.0.0" && sha256sum "nurt-3.0.0-$target.tar.gz" >checksums.txt)
run_install 3.0.0 "$work/bin-nested"
expect 'installs an archive with the top directory of the release layout' succeeded
expect 'executable of the nested archive runs' prints "$work/bin-nested/nurt" 'Danaco Nurt 3.0.0'

printf '%064d  nurt-1.2.3-%s.tar.gz\n' 0 "$target" >"$work/release/v1.2.3/checksums.txt"
run_install 1.2.3 "$work/bin-tampered"
expect 'fails on checksum mismatch' failed_with 'checksum mismatch'
expect 'installs nothing after a checksum mismatch' missing "$work/bin-tampered/nurt"

expect 'maps the supported architectures' prints_target
expect 'refuses an unknown architecture' refused_architecture riscv64

((failures == 0))
