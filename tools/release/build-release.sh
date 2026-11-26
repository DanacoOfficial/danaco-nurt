#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Release build.
#
# Builds the executable and the C library for Linux (glibc, and musl when the
# target is installed) and for Windows (MSVC, cross-built with cargo-xwin),
# generates the manual pages, the shell completions and the documentation
# site, assembles one archive per target with packaging/release/pack.py,
# writes checksums.txt, optionally signs it, and runs the executable of the
# Linux archive.
#
# Usage:
#
#   tools/release/build-release.sh <version> <output-dir>
#       [--sign <gnupg-home> <key-id>] [--lib-archive] [--no-windows]
#
#   --sign         sign checksums.txt with the key <key-id> of the GnuPG home
#                  <gnupg-home> (checksums.txt.asc) and verify the signature
#   --lib-archive  also assemble the archive of the nurt-lib library with the
#                  command defined in lib_pack_command
#   --no-windows   skip the Windows target
#
# Environment (all optional):
#
#   NURT_RELEASE_MUSL   auto (default), 1 or 0: build the musl target
#   SOURCE_DATE_EPOCH   time of the archive entries; the commit time of HEAD
#                       when unset
#   CARGO_TARGET_DIR    build directory; the target directory of the workspace
#                       when unset
#
# The output directory receives the archives, checksums.txt and, with --sign,
# checksums.txt.asc. The directory must not exist or must be empty. Exit code
# 0 means that every step succeeded.

set -euo pipefail

LINUX_GNU=x86_64-unknown-linux-gnu
LINUX_MUSL=x86_64-unknown-linux-musl
WINDOWS=x86_64-pc-windows-msvc

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

usage() {
  sed -n '/^# Usage:/,/^# The output/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

log() { printf '[build-release] %s\n' "$*" >&2; }
die() { printf '[build-release] error: %s\n' "$*" >&2; exit 1; }

version=""
output=""
sign=0
gnupg_home=""
key_id=""
lib_archive=0
windows=1

while [ $# -gt 0 ]; do
  case "$1" in
    --sign)
      [ $# -ge 3 ] || usage
      sign=1
      gnupg_home="$2"
      key_id="$3"
      shift 3
      ;;
    --lib-archive) lib_archive=1; shift ;;
    --no-windows) windows=0; shift ;;
    -h | --help) usage ;;
    -*) usage ;;
    *)
      if [ -z "$version" ]; then version="$1"
      elif [ -z "$output" ]; then output="$1"
      else usage
      fi
      shift
      ;;
  esac
done
[ -n "$version" ] && [ -n "$output" ] || usage
printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$' \
  || die "invalid version: $version"

if [ -e "$output" ] && [ -n "$(ls -A "$output")" ]; then
  die "output directory is not empty: $output"
fi
mkdir -p "$output"
output="$(cd "$output" && pwd)"
work="$output/.work"
mkdir -p "$work"
trap 'rm -rf "$work"' EXIT

cd "$root"

epoch="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct HEAD)}"
export SOURCE_DATE_EPOCH="$epoch"
date_text="$(date -u -d "@$epoch" +%Y-%m-%d)"

target_dir="${CARGO_TARGET_DIR:-$(cargo metadata --no-deps --format-version 1 \
  | python3 -I -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')}"

targets=("$LINUX_GNU")
case "${NURT_RELEASE_MUSL:-auto}" in
  1) targets+=("$LINUX_MUSL") ;;
  0) ;;
  auto)
    if rustup target list --installed 2>/dev/null | grep -qx "$LINUX_MUSL"; then
      targets+=("$LINUX_MUSL")
    fi
    ;;
  *) die "NURT_RELEASE_MUSL must be auto, 1 or 0" ;;
esac
if [ "$windows" -eq 1 ]; then targets+=("$WINDOWS"); fi
log "version $version, targets: ${targets[*]}"

# Archive of the nurt-lib library, run only with --lib-archive. The split of
# the library archive (#58) is not merged: --kind is a placeholder that pack.py
# does not accept yet. Adjust only this function after the merge of #58.
# Arguments: target, library file, output directory.
lib_pack_command() {
  python3 -I "$root/packaging/release/pack.py" --root "$root" --version "$version" \
    --target "$1" --binary "$2" --library "$2" --output "$3" --kind library
}

build_target() {
  case "$1" in
    "$WINDOWS") cargo xwin build --release --locked --target "$1" -p nurt-cli -p nurt-ffi ;;
    *) cargo build --release --locked --target "$1" -p nurt-cli -p nurt-ffi ;;
  esac
}

library_file() {
  local dir="$target_dir/$1/release"
  case "$1" in
    *-windows-*) echo "$dir/nurt_ffi.dll" ;;
    *-apple-darwin) echo "$dir/libnurt_ffi.dylib" ;;
    *) echo "$dir/libnurt_ffi.so" ;;
  esac
}

log "documentation tools"
cargo build --release --locked -p nurt-docgen
docgen="$target_dir/release/nurt-docgen"
[ -x "$docgen" ] || die "missing $docgen"
"$docgen" man --root "$root" --out "$work/man" --date "$date_text" --version "$version"
"$docgen" completions --root "$root" --out "$work/completions"
"$docgen" site --root "$root" --out "$work/site" --version "$version"

for target in "${targets[@]}"; do
  log "build $target"
  build_target "$target"
  binary="$target_dir/$target/release/nurt"
  if [ "$target" = "$WINDOWS" ]; then binary="$binary.exe"; fi
  [ -f "$binary" ] || die "missing $binary"
  library_args=()
  library=""
  # The musl executable is static and has no library.
  if [ "$target" != "$LINUX_MUSL" ]; then
    library="$(library_file "$target")"
    [ -f "$library" ] || die "missing $library"
    library_args=(--library "$library")
  fi
  man_args=(--man "$work/man")
  if [ "$target" = "$WINDOWS" ]; then man_args=(); fi
  log "pack $target"
  python3 -I "$root/packaging/release/pack.py" --root "$root" --version "$version" \
    --target "$target" --binary "$binary" "${library_args[@]}" "${man_args[@]}" \
    --completions "$work/completions" --site "$work/site" --output "$output"
  if [ "$lib_archive" -eq 1 ] && [ -n "$library" ]; then
    lib_pack_command "$target" "$library" "$output"
  fi
done

log "checksums"
(
  cd "$output"
  files=()
  for file in nurt-*.tar.gz nurt-*.zip; do
    if [ -e "$file" ]; then files+=("$file"); fi
  done
  [ "${#files[@]}" -gt 0 ] || die "no archives were written"
  sha256sum "${files[@]}" > checksums.txt
  sha256sum --check --strict checksums.txt
)

if [ "$sign" -eq 1 ]; then
  log "sign checksums.txt"
  export GNUPGHOME="$gnupg_home"
  gpg --batch --yes --armor --detach-sign -u "$key_id" \
    --output "$output/checksums.txt.asc" "$output/checksums.txt"
  gpg --verify "$output/checksums.txt.asc" "$output/checksums.txt"
else
  log "signing skipped"
fi

log "run the executable of the Linux archive"
run="$work/run"
mkdir -p "$run/home"
tar -xzf "$output/nurt-$version-$LINUX_GNU.tar.gz" -C "$run"
nurt="$run/nurt-$version-$LINUX_GNU/nurt"
[ -x "$nurt" ] || die "the archive has no executable $nurt"
(
  cd "$run"
  export HOME="$run/home" XDG_STATE_HOME="$run/home/state" XDG_CONFIG_HOME="$run/home/config"
  export LD_LIBRARY_PATH="$run/nurt-$version-$LINUX_GNU"
  "$nurt" version
  "$nurt" doctor
)

log "done: $(ls "$output" | tr '\n' ' ')"
