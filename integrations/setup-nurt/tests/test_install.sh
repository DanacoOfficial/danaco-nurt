#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           integrations/setup-nurt/tests/test_install.sh
# Responsibility:   End-to-end tests of setup-nurt.sh against a release tree in a temporary directory, addressed through file:// URLs.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.2, 2026-11-28
# License:          see the LICENSE file or https://www.danaco-group.pl

# End-to-end tests of setup-nurt.sh against a release tree in a temporary
# directory, addressed through file:// URLs.

set -uo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=harness.sh
source "$here/harness.sh"
setup="$here/../scripts/setup-nurt.sh"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

release="$work/release"
target=x86_64-unknown-linux-musl

# make_release VERSION: archive with a stand-in executable and a checksum list.
make_release() {
  local version=$1 dir="$release/v$1" stage
  stage=$(mktemp -d "$work/stage.XXXXXX")
  mkdir -p "$dir"
  printf '#!/bin/sh\necho "Danaco Nurt %s"\n' "$version" >"$stage/nurt"
  chmod +x "$stage/nurt"
  printf 'notice\n' >"$stage/NOTICE"
  tar -czf "$dir/nurt-$version-$target.tar.gz" -C "$stage" nurt NOTICE
  (cd "$dir" && {
    if command -v sha256sum >/dev/null 2>&1; then
      sha256sum "nurt-$version-$target.tar.gz"
    else
      shasum -a 256 "nurt-$version-$target.tar.gz"
    fi
  } >checksums.txt)
}

# run_setup VERSION VERIFY CACHE: runs the script and captures its output files.
run_setup() {
  : >"$work/github_output"
  : >"$work/github_path"
  env RUNNER_OS=Linux RUNNER_ARCH=X64 RUNNER_TOOL_CACHE="$3" \
    GITHUB_OUTPUT="$work/github_output" GITHUB_PATH="$work/github_path" \
    NURT_DOWNLOAD_BASE="$(file_url "$release")" NURT_RELEASES_URL="$(file_url "$work/releases.json")" \
    INPUT_VERSION="$1" INPUT_VERIFY="$2" \
    bash "$setup" >"$work/stdout" 2>"$work/stderr"
}

make_release 1.2.3
make_release 1.4.0
cat >"$work/releases.json" <<'JSON'
[{"tag_name": "v1.4.0"}, {"tag_name": "v1.2.3"}, {"tag_name": "v1.5.0-rc.1"}]
JSON

cache="$work/cache-a"
run_setup 1.2.3 true "$cache"
assert_eq 'installs an exact version with verification' 0 $?
assert_file 'places the executable in the tool cache' "$cache/nurt/1.2.3/x64/nurt"
assert_file 'writes the completion marker' "$cache/nurt/1.2.3/x64.complete"
assert_file 'keeps the notice file from the archive' "$cache/nurt/1.2.3/x64/NOTICE"
assert_contains 'reports verification' 'verified SHA-256' "$(cat "$work/stdout")"
assert_contains 'outputs the executable path' "path=$(native_path "$cache/nurt/1.2.3/x64/nurt")" "$(cat "$work/github_output")"
assert_contains 'outputs the resolved version' 'version=1.2.3' "$(cat "$work/github_output")"
assert_eq 'adds the install directory to the path' "$(native_path "$cache/nurt/1.2.3/x64")" "$(cat "$work/github_path")"
assert_eq 'installed executable runs' 'Danaco Nurt 1.2.3' "$("$cache/nurt/1.2.3/x64/nurt")"

rm -rf "$release/v1.2.3"
run_setup 1.2.3 true "$cache"
assert_eq 'reuses the cached version without downloading' 0 $?
assert_contains 'reports the cache hit' 'using cached Danaco Nurt 1.2.3' "$(cat "$work/stdout")"
make_release 1.2.3

cache="$work/cache-b"
run_setup latest true "$cache"
assert_eq 'resolves latest to the highest stable release' 0 $?
assert_contains 'latest ignores the pre-release' 'version=1.4.0' "$(cat "$work/github_output")"

cache="$work/cache-c"
run_setup '~1.2.0' true "$cache"
assert_eq 'resolves a range' 0 $?
assert_contains 'range selects the matching release' 'version=1.2.3' "$(cat "$work/github_output")"

cache="$work/cache-d"
printf 'tampered' >>"$release/v1.2.3/nurt-1.2.3-$target.tar.gz"
run_setup 1.2.3 true "$cache"
assert_eq 'fails on checksum mismatch' 1 $?
assert_contains 'explains the mismatch' 'checksum mismatch' "$(cat "$work/stderr")"
assert_no_file 'leaves no completion marker after a failure' "$cache/nurt/1.2.3/x64.complete"
assert_no_file 'leaves no executable after a failure' "$cache/nurt/1.2.3/x64/nurt"

make_release 1.2.3
printf '%064d  nurt-1.2.3-%s.tar.gz\n' 0 "$target" >"$release/v1.2.3/checksums.txt"
run_setup 1.2.3 false "$cache"
assert_eq 'installs without verification when disabled' 0 $?
assert_file 'installs the executable when verification is disabled' "$cache/nurt/1.2.3/x64/nurt"
make_release 1.2.3

cache="$work/cache-e"
rm "$release/v1.2.3/checksums.txt"
run_setup 1.2.3 true "$cache"
assert_eq 'fails when the checksum list is missing' 1 $?
assert_contains 'names the missing checksum list' 'checksums.txt' "$(cat "$work/stderr")"
make_release 1.2.3

cache="$work/cache-f"
run_setup 9.9.9 true "$cache"
assert_eq 'fails when the release is missing' 1 $?
assert_contains 'names the missing archive' 'cannot download' "$(cat "$work/stderr")"

run_setup '>=7.0.0' true "$cache"
assert_eq 'fails when no release satisfies the range' 1 $?
assert_contains 'reports the unsatisfied range' "no release satisfies version '>=7.0.0'" "$(cat "$work/stderr")"

run_setup 1.2.3 maybe "$cache"
assert_eq 'rejects an invalid verify value' 1 $?
assert_contains 'explains the verify values' "input 'verify'" "$(cat "$work/stderr")"

cache="$work/cache-g"
stage=$(mktemp -d "$work/stage.XXXXXX")
printf 'x\n' >"$stage/README"
tar -czf "$release/v1.2.3/nurt-1.2.3-$target.tar.gz" -C "$stage" README
(cd "$release/v1.2.3" && { sha256sum "nurt-1.2.3-$target.tar.gz" 2>/dev/null || shasum -a 256 "nurt-1.2.3-$target.tar.gz"; } >checksums.txt)
run_setup 1.2.3 true "$cache"
assert_eq 'fails when the archive lacks the executable' 1 $?
assert_contains 'names the missing executable' 'does not contain nurt' "$(cat "$work/stderr")"

cache="$work/cache-i"
stage=$(mktemp -d "$work/stage.XXXXXX")
mkdir -p "$stage/nurt-1.2.3-$target"
printf '#!/bin/sh\necho "nurt 1.2.3"\n' >"$stage/nurt-1.2.3-$target/nurt"
chmod +x "$stage/nurt-1.2.3-$target/nurt"
tar -czf "$release/v1.2.3/nurt-1.2.3-$target.tar.gz" -C "$stage" "nurt-1.2.3-$target"
(cd "$release/v1.2.3" && { sha256sum "nurt-1.2.3-$target.tar.gz" 2>/dev/null || shasum -a 256 "nurt-1.2.3-$target.tar.gz"; } >checksums.txt)
run_setup 1.2.3 true "$cache"
assert_eq 'installs an archive with the top directory of the release layout' 0 $?
assert_file 'places the executable of the nested archive in the tool cache' "$cache/nurt/1.2.3/x64/nurt"

cache="$work/cache-j"
gnu=x86_64-unknown-linux-gnu
rm -rf "$release/v1.2.3"
stage=$(mktemp -d "$work/stage.XXXXXX")
mkdir -p "$release/v1.2.3" "$stage/nurt-1.2.3-$gnu"
printf '#!/bin/sh\necho "nurt 1.2.3"\n' >"$stage/nurt-1.2.3-$gnu/nurt"
chmod +x "$stage/nurt-1.2.3-$gnu/nurt"
tar -czf "$release/v1.2.3/nurt-1.2.3-$gnu.tar.gz" -C "$stage" "nurt-1.2.3-$gnu"
(cd "$release/v1.2.3" && { sha256sum "nurt-1.2.3-$gnu.tar.gz" 2>/dev/null || shasum -a 256 "nurt-1.2.3-$gnu.tar.gz"; } >checksums.txt)
run_setup 1.2.3 true "$cache"
assert_eq 'installs the glibc archive of a release without a musl archive' 0 $?
assert_contains 'reports the missing musl archive' 'no musl archive in version 1.2.3' "$(cat "$work/stdout")"
assert_contains 'verifies the glibc archive' "verified SHA-256 of nurt-1.2.3-$gnu.tar.gz" "$(cat "$work/stdout")"
assert_file 'places the executable of the glibc archive in the tool cache' "$cache/nurt/1.2.3/x64/nurt"
make_release 1.2.3

cache="$work/cache-h"
env RUNNER_OS=FreeBSD RUNNER_ARCH=X64 RUNNER_TOOL_CACHE="$cache" INPUT_VERSION=1.2.3 \
  bash "$setup" >/dev/null 2>"$work/stderr"
assert_eq 'fails on an unsupported system' 1 $?
assert_contains 'names the unsupported system' "unsupported operating system 'FreeBSD'" "$(cat "$work/stderr")"

finish_tests

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
