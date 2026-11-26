#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Tests for artifact selection and SHA-256 verification.

set -uo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=harness.sh
source "$here/harness.sh"
# shellcheck source=../scripts/lib.sh
source "$here/../scripts/lib.sh"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

assert_eq 'Linux X64 target' x86_64-unknown-linux-musl "$(release_target Linux X64)"
assert_eq 'Linux ARM64 target' aarch64-unknown-linux-musl "$(release_target Linux ARM64)"
assert_eq 'macOS X64 target' x86_64-apple-darwin "$(release_target macOS X64)"
assert_eq 'macOS ARM64 target' aarch64-apple-darwin "$(release_target macOS ARM64)"
assert_eq 'Windows X64 target' x86_64-pc-windows-msvc "$(release_target Windows X64)"
assert_eq 'Windows ARM64 target' aarch64-pc-windows-msvc "$(release_target Windows ARM64)"
assert_eq 'uname Linux x86_64 target' x86_64-unknown-linux-musl "$(release_target Linux x86_64)"
assert_eq 'uname Darwin arm64 target' aarch64-apple-darwin "$(release_target Darwin arm64)"
assert_eq 'uname MSYS target' x86_64-pc-windows-msvc "$(release_target MINGW64_NT-10.0 AMD64)"
# lib_run FUNCTION ARGUMENT...: runs a function of lib.sh in a new shell; the path
# and the arguments are passed as words, so no quoting depends on their text.
lib_run() {
  bash -c 'source "$1"; shift; "$@"' _ "$here/../scripts/lib.sh" "$@"
}

assert_fails 'rejects unknown system' lib_run release_target FreeBSD X64
out=$(lib_run release_target FreeBSD X64 2>&1 || true)
assert_contains 'names the unknown system' "unsupported operating system 'FreeBSD'" "$out"
assert_fails 'rejects unknown architecture' lib_run release_target Linux riscv64
out=$(lib_run release_target Linux riscv64 2>&1 || true)
assert_contains 'names the unknown architecture' "unsupported architecture 'riscv64'" "$out"
assert_fails 'rejects unknown cache architecture' lib_run cache_arch riscv64

assert_eq 'tool-cache name for X64' x64 "$(cache_arch X64)"
assert_eq 'tool-cache name for aarch64' arm64 "$(cache_arch aarch64)"

assert_eq 'tar archive on Linux' nurt-1.2.3-x86_64-unknown-linux-musl.tar.gz \
  "$(archive_name 1.2.3 x86_64-unknown-linux-musl)"
assert_eq 'tar archive on macOS' nurt-1.2.3-aarch64-apple-darwin.tar.gz \
  "$(archive_name 1.2.3 aarch64-apple-darwin)"
assert_eq 'zip archive on Windows' nurt-1.2.3-x86_64-pc-windows-msvc.zip \
  "$(archive_name 1.2.3 x86_64-pc-windows-msvc)"

printf 'abc' >"$work/data.bin"
abc_digest=ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
assert_eq 'sha256 of known input' "$abc_digest" "$(sha256_of "$work/data.bin")"

printf '%s  data.bin\n' "$abc_digest" >"$work/sums-ok"
assert_succeeds 'accepts matching checksum' verify_checksum "$work/data.bin" data.bin "$work/sums-ok"

printf '%s  data.bin\n' "$(printf '%s' "$abc_digest" | tr 'a-f' 'A-F')" >"$work/sums-upper"
assert_succeeds 'accepts upper-case digest' verify_checksum "$work/data.bin" data.bin "$work/sums-upper"

printf '%s *data.bin\n' "$abc_digest" >"$work/sums-binary"
assert_succeeds 'accepts binary-mode marker' verify_checksum "$work/data.bin" data.bin "$work/sums-binary"

printf '%064d  data.bin\n' 0 >"$work/sums-bad"
out=$( (verify_checksum "$work/data.bin" data.bin "$work/sums-bad") 2>&1 || true)
assert_contains 'reports checksum mismatch' 'checksum mismatch for data.bin' "$out"
assert_fails 'rejects wrong checksum' lib_run verify_checksum "$work/data.bin" data.bin "$work/sums-bad"

printf '%s  other.bin\n' "$abc_digest" >"$work/sums-other"
out=$( (verify_checksum "$work/data.bin" data.bin "$work/sums-other") 2>&1 || true)
assert_contains 'reports missing checksum entry' 'no checksum listed for data.bin' "$out"

finish_tests
