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
# site, assembles the program archive and the archive of the C library nurt-lib
# of every target with packaging/release/pack.py (the library of the musl
# target is the static library libnurt.a), writes checksums.txt, optionally
# signs it, runs the executable of the Linux archive and builds and runs
# examples/c/hello.c against the library archive of the Linux target.
#
# Usage:
#
#   tools/release/build-release.sh <version> <output-dir>
#       [--sign <gnupg-home> <key-id>] [--no-windows]
#
#   --sign         sign checksums.txt with the key <key-id> of the GnuPG home
#                  <gnupg-home> (checksums.txt.asc) and verify the signature
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
    *-linux-musl) echo "$dir/libnurt_ffi.a" ;;
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
  library="$(library_file "$target")"
  [ -f "$library" ] || die "missing $library"
  library_args=(--library "$library")
  man_args=(--man "$work/man")
  if [ "$target" = "$WINDOWS" ]; then man_args=(); fi
  log "pack $target"
  python3 -I "$root/packaging/release/pack.py" --root "$root" --version "$version" \
    --target "$target" --binary "$binary" "${library_args[@]}" "${man_args[@]}" \
    --completions "$work/completions" --site "$work/site" --output "$output"
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

# Builds examples/c/hello.c against the archive nurt-lib of a Linux target with
# the flags of its nurt.pc and runs the program. The musl library is static, so
# the program is linked with --static and the compiler of the musl target.
# Arguments: target, C compiler, extra compiler flags.
check_c_example() {
  local target="$1" compiler="$2" extra="$3" dir="$work/c-example/$1"
  local lib="$dir/nurt-lib-$version-$target" static=()
  command -v pkg-config >/dev/null || die "pkg-config is required for the C example"
  mkdir -p "$dir/lib"
  tar -xzf "$output/nurt-lib-$version-$target.tar.gz" -C "$dir"
  for name in libnurt.so.1 libnurt.a; do
    if [ -e "$lib/$name" ]; then ln -sf "$lib/$name" "$dir/lib/$name"; fi
  done
  if [ -e "$dir/lib/libnurt.so.1" ]; then ln -sf libnurt.so.1 "$dir/lib/libnurt.so"; fi
  case "$target" in *-linux-musl) static=(--static) ;; esac
  local flags
  flags="$(PKG_CONFIG_PATH="$lib" pkg-config --define-variable=libdir="$dir/lib" \
    --define-variable=includedir="$lib" ${static[@]+"${static[@]}"} --cflags --libs nurt)"
  # The flags are words of one command line; the paths hold no spaces.
  # shellcheck disable=SC2086
  $compiler $extra "$root/examples/c/hello.c" $flags -o "$dir/hello"
  LD_LIBRARY_PATH="$dir/lib" "$dir/hello" | tee "$dir/hello.out"
  grep -Eqx 'nurt [0-9]+\.[0-9]+\.[0-9]+.*' "$dir/hello.out" || die "the C example of $target printed no version"
}

log "build and run the C example against the library of the Linux archive"
check_c_example "$LINUX_GNU" cc ""
for target in "${targets[@]}"; do
  if [ "$target" = "$LINUX_MUSL" ]; then
    musl_cc="${CC_x86_64_unknown_linux_musl:-musl-gcc}"
    check_c_example "$LINUX_MUSL" "$musl_cc" "-static"
  fi
done

log "done: $(ls "$output" | tr '\n' ' ')"
