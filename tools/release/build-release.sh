#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           tools/release/build-release.sh
# Responsibility:   Builds the archives of the executable and of nurt-lib for every target and writes their checksums.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.2, 2026-11-28
# License:          see the LICENSE file or https://www.danaco-group.pl

#
# Release build.
#
# Builds the executable and the C library nurt-lib for Linux x86_64 (glibc, and
# musl when the target is installed), Linux aarch64 (glibc and musl, when the
# targets are installed and zig is available) and Windows (x86_64 and aarch64
# MSVC, cross-built with cargo-xwin), generates the manual pages, the shell
# completions and the documentation site, assembles the program archive and the
# archive of nurt-lib of every target with packaging/release/pack.py (the
# library of a musl target is the static library libnurt.a without the debug
# information and the embedded bitcode), builds the Windows setup wizard of every
# Windows target around the program archive of that target
# (nurt-<version>-<target>-setup.exe), writes checksums.txt, optionally signs it,
# and, for the target of the host only, runs the executable of its archive and
# builds and runs examples/c/hello.c against the library archive.
#
# The Linux targets that are not built with the C compiler of the host are built
# with zig cc: the scripts packaging/release/zig-cc-*.sh and zig-ar.sh set the
# target of zig and drop the options of rustc and of the cc crate that zig
# rejects. The script sets CC_<target>, AR_<target>, CFLAGS_<target>=
# -fno-sanitize=undefined and, for aarch64, CARGO_TARGET_<TARGET>_LINKER, unless
# the caller has set them.
#
# The setup wizard (packaging/windows-installer, its own Cargo.lock, outside the
# workspace) embeds the program archive of its target. The script passes the
# archive in NURT_SETUP_PAYLOAD and the sum that sha256sum reports for it in
# NURT_SETUP_PAYLOAD_SHA256, so that the archive in checksums.txt and the
# archive in the wizard are the same file. The version of the package nurt-setup
# must equal <version>; the script stops when it does not.
#
# Usage:
#
#   tools/release/build-release.sh <version> <output-dir>
#       [--sign <gnupg-home> <key-id>] [--codesign <p12-file> <password-file>]
#       [--no-windows] [--no-setup]
#
#   --sign         sign checksums.txt with the key <key-id> of the GnuPG home
#                  <gnupg-home> (checksums.txt.asc) and verify the signature
#   --codesign     sign nurt.exe before it is packed and every setup wizard
#                  after it is built with Authenticode (osslsigncode), using
#                  the PKCS#12 file <p12-file> and the password that the file
#                  <password-file> holds
#   --no-windows   skip the Windows targets (and so the setup wizards)
#   --no-setup     build the Windows archives but not the setup wizards
#
# Environment (all optional):
#
#   NURT_RELEASE_MUSL   auto (default), 1 or 0: build the musl target of x86_64
#                       (auto: when the target is installed)
#   NURT_RELEASE_AARCH64  auto (default), 1 or 0: build the aarch64 Linux
#                       targets (glibc and musl); auto needs both installed
#                       targets and zig
#   ZIG                 the zig executable of the zig cc wrappers; zig when unset
#   NURT_MSVC_CLANG     the clang executable of the MSVC wrappers of
#                       packaging/release/msvc-clang; /usr/bin/clang when unset
#   NURT_RC             the resource compiler of the setup wizard; when unset
#                       and neither llvm-rc nor rc is in PATH, the llvm-rc next
#                       to the clang of NURT_MSVC_CLANG
#   NURT_OSSLSIGNCODE   the osslsigncode executable of --codesign;
#                       osslsigncode when unset
#   SOURCE_DATE_EPOCH   time of the archive entries; the commit time of HEAD
#                       when unset
#   CARGO_TARGET_DIR    build directory; the target directory of the workspace
#                       when unset
#
# The output directory receives the archives, the setup wizards, checksums.txt
# and, with --sign, checksums.txt.asc. The directory must not exist or must be
# empty. Exit code 0 means that every step succeeded.
#
# The password file of --codesign is read by osslsigncode only; its content is
# neither printed nor passed in an argument. The time stamp of a signature comes
# from the first of three servers that answers (RFC 3161); when none answers,
# the files are signed without a time stamp and the script says so.

set -euo pipefail

LINUX_GNU=x86_64-unknown-linux-gnu
LINUX_MUSL=x86_64-unknown-linux-musl
LINUX_AARCH64_GNU=aarch64-unknown-linux-gnu
LINUX_AARCH64_MUSL=aarch64-unknown-linux-musl
WINDOWS=x86_64-pc-windows-msvc
WINDOWS_ARM=aarch64-pc-windows-msvc

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
setup=1
codesign=0
codesign_p12=""
codesign_pass=""

while [ $# -gt 0 ]; do
  case "$1" in
    --sign)
      [ $# -ge 3 ] || usage
      sign=1
      gnupg_home="$2"
      key_id="$3"
      shift 3
      ;;
    --codesign)
      [ $# -ge 3 ] || usage
      codesign=1
      codesign_p12="$2"
      codesign_pass="$3"
      shift 3
      ;;
    --no-windows) windows=0; shift ;;
    --no-setup) setup=0; shift ;;
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

osslsigncode="${NURT_OSSLSIGNCODE:-osslsigncode}"
if [ "$codesign" -eq 1 ]; then
  [ -f "$codesign_p12" ] || die "the PKCS#12 file of --codesign does not exist: $codesign_p12"
  [ -f "$codesign_pass" ] || die "the password file of --codesign does not exist: $codesign_pass"
  command -v "$osslsigncode" >/dev/null 2>&1 || die "--codesign needs osslsigncode (set NURT_OSSLSIGNCODE)"
fi

# The version of the package of the setup wizard.
setup_manifest="$root/packaging/windows-installer/Cargo.toml"
if [ "$windows" -eq 1 ] && [ "$setup" -eq 1 ]; then
  setup_version="$(sed -n '/^\[package\]/,/^\[/{s/^version *= *"\([^"]*\)".*/\1/p}' "$setup_manifest" | head -n 1)"
  [ -n "$setup_version" ] || die "no version in $setup_manifest"
  [ "$setup_version" = "$version" ] \
    || die "the version of the package nurt-setup is $setup_version, not $version (packaging/windows-installer/Cargo.toml)"
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

host="$(rustc -vV | sed -n 's/^host: //p')"
zig_command="${ZIG:-zig}"
installed_targets="$(rustup target list --installed 2>/dev/null || true)"

target_installed() { printf '%s\n' "$installed_targets" | grep -qx "$1"; }

zig_available() { command -v "$zig_command" >/dev/null 2>&1; }

targets=("$LINUX_GNU")
case "${NURT_RELEASE_MUSL:-auto}" in
  1) targets+=("$LINUX_MUSL") ;;
  0) ;;
  auto) if target_installed "$LINUX_MUSL"; then targets+=("$LINUX_MUSL"); fi ;;
  *) die "NURT_RELEASE_MUSL must be auto, 1 or 0" ;;
esac
case "${NURT_RELEASE_AARCH64:-auto}" in
  1) targets+=("$LINUX_AARCH64_GNU" "$LINUX_AARCH64_MUSL") ;;
  0) ;;
  auto)
    if target_installed "$LINUX_AARCH64_GNU" && target_installed "$LINUX_AARCH64_MUSL" && zig_available; then
      targets+=("$LINUX_AARCH64_GNU" "$LINUX_AARCH64_MUSL")
    fi
    ;;
  *) die "NURT_RELEASE_AARCH64 must be auto, 1 or 0" ;;
esac
if [ "$windows" -eq 1 ]; then targets+=("$WINDOWS" "$WINDOWS_ARM"); fi
log "version $version, targets: ${targets[*]}"

# Sets the variable $1 to $2 unless the caller has set it.
export_default() {
  if [ -z "${!1:-}" ]; then export "$1=$2"; fi
}

# Environment of the cargo build of a Linux target that is built with zig cc.
zig_environment() {
  local target="$1" wrapper="" linker=0 name upper
  case "$target" in
    "$LINUX_MUSL") wrapper=zig-cc-musl.sh ;;
    "$LINUX_AARCH64_GNU") wrapper=zig-cc-aarch64-linux-gnu.sh; linker=1 ;;
    "$LINUX_AARCH64_MUSL") wrapper=zig-cc-aarch64-linux-musl.sh; linker=1 ;;
    *) return 0 ;;
  esac
  if ! zig_available; then
    # The musl target of x86_64 can use another C compiler (musl-gcc).
    [ "$target" = "$LINUX_MUSL" ] && return 0
    die "zig is required to build $target (set ZIG)"
  fi
  export ZIG="$zig_command"
  name="${target//-/_}"
  upper="$(printf '%s' "$name" | tr '[:lower:]' '[:upper:]')"
  export_default "CC_$name" "$root/packaging/release/$wrapper"
  export_default "CFLAGS_$name" "-fno-sanitize=undefined"
  if [ "$linker" -eq 1 ]; then
    export_default "AR_$name" "$root/packaging/release/zig-ar.sh"
    export_default "CARGO_TARGET_${upper}_LINKER" "$root/packaging/release/$wrapper"
  fi
}

build_target() {
  case "$1" in
    "$WINDOWS" | "$WINDOWS_ARM")
      # The wrappers of packaging/release/msvc-clang translate the options
      # of cargo-xwin that clang (ring, aarch64 assembly) and clang-cl (aws-lc,
      # /FI with an absolute path) do not take from a Linux host.
      PATH="$root/packaging/release/msvc-clang:$PATH" \
        cargo xwin build --release --locked --target "$1" -p nurt-cli -p nurt-ffi ;;
    *)
      zig_environment "$1"
      cargo build --release --locked --target "$1" -p nurt-cli -p nurt-ffi ;;
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

# Prints the objcopy of LLVM, which reads the archives of every target:
# LLVM_OBJCOPY, llvm-objcopy in PATH, or rust-objcopy of the component
# llvm-tools of the toolchain.
llvm_objcopy() {
  local tool
  if [ -n "${LLVM_OBJCOPY:-}" ]; then echo "$LLVM_OBJCOPY"; return 0; fi
  if command -v llvm-objcopy >/dev/null 2>&1; then command -v llvm-objcopy; return 0; fi
  tool="$(rustc --print sysroot)/lib/rustlib/$host/bin/rust-objcopy"
  if [ -x "$tool" ]; then echo "$tool"; return 0; fi
  return 1
}

# Copies the static library $1 of a musl target to $2 without what a link does
# not use: the debug information and the LLVM bitcode that rustc embeds in every
# object for link-time optimization (the sections .llvmbc and .llvmcmd, more
# than half of the file). The symbol tables stay, so the exported functions of
# nurt.symbols remain defined.
shrink_static_library() {
  local source="$1" copy="$2" tool
  tool="$(llvm_objcopy)" || die "llvm-objcopy is required to shrink the static library (set LLVM_OBJCOPY)"
  cp "$source" "$copy"
  "$tool" --strip-debug --remove-section=.llvmbc --remove-section=.llvmcmd "$copy"
  log "static library: $(stat -c %s "$source") bytes, shrunk to $(stat -c %s "$copy") bytes"
}

# RFC 3161 servers of the time stamp of an Authenticode signature, in the order
# of preference.
timestamp_servers=(http://timestamp.digicert.com http://time.certum.pl http://timestamp.sectigo.com)
timestamp_server=""
codesign_ready=0

# Picks the first time stamp server that accepts a connection; sets
# timestamp_server, which stays empty when none does.
pick_timestamp_server() {
  local url
  if command -v curl >/dev/null 2>&1; then
    for url in "${timestamp_servers[@]}"; do
      if curl -sS -m 8 -o /dev/null "$url" 2>/dev/null; then
        timestamp_server="$url"
        log "time stamp server: $url"
        return 0
      fi
    done
  fi
  log "warning: no time stamp server answers; the files are signed without a time stamp"
}

# Checks the Authenticode signature of a file against the public certificate of
# the repository. A missing signature stops the script; a signature whose chain
# is not trusted (a self-signed certificate) is reported as a warning.
# Argument: file.
verify_signature() {
  local file="$1" report="$work/verify.log" ca=()
  if [ -f "$root/packaging/release/danaco-nurt-codesign.pem" ]; then
    ca=(-CAfile "$root/packaging/release/danaco-nurt-codesign.pem")
  fi
  if "$osslsigncode" verify ${ca[@]+"${ca[@]}"} -in "$file" >"$report" 2>&1; then
    log "signature verified: $(basename "$file")"
    return 0
  fi
  if grep -qiE 'no signature found|not signed' "$report" || ! grep -qi 'signature' "$report"; then
    cat "$report" >&2
    die "the file has no Authenticode signature: $file"
  fi
  log "warning: the signature of $(basename "$file") is present but its chain is not trusted"
}

# Signs a file with Authenticode in place and verifies the signature. The
# password is read by osslsigncode from the password file; it is not an
# argument and no command line is logged. Argument: file.
sign_file() {
  local file="$1" signed="$1.signed" log_file="$work/osslsigncode.log"
  local arguments=(sign -pkcs12 "$codesign_p12" -readpass "$codesign_pass" -h sha256
    -n "Danaco Nurt" -i https://github.com/DanacoOfficial/danaco-nurt)
  if [ "$codesign_ready" -eq 0 ]; then
    pick_timestamp_server
    codesign_ready=1
  fi
  if [ -n "$timestamp_server" ]; then arguments+=(-ts "$timestamp_server"); fi
  log "sign $(basename "$file")"
  if ! "$osslsigncode" "${arguments[@]}" -in "$file" -out "$signed" >"$log_file" 2>&1; then
    cat "$log_file" >&2
    rm -f "$signed"
    die "osslsigncode failed to sign $(basename "$file")"
  fi
  mv -f "$signed" "$file"
  verify_signature "$file"
}

# Makes a resource compiler available to the build script of the setup wizard:
# NURT_RC, or llvm-rc or rc in PATH, or the llvm-rc next to the clang of the MSVC
# wrappers.
resource_compiler_environment() {
  local clang_dir
  if [ -n "${NURT_RC:-}" ] || command -v llvm-rc >/dev/null 2>&1 || command -v rc >/dev/null 2>&1; then
    return 0
  fi
  clang_dir="$(dirname "${NURT_MSVC_CLANG:-/usr/bin/clang}")"
  if [ -x "$clang_dir/llvm-rc" ]; then
    export NURT_RC="$clang_dir/llvm-rc"
  fi
}

# Builds the setup wizard of a Windows target around the program archive of the
# target and copies it to the output directory. Argument: target.
build_setup() {
  local target="$1" archive="$output/nurt-$version-$1.zip" sum built
  [ -f "$archive" ] || die "missing $archive"
  sum="$(sha256sum "$archive" | cut -d ' ' -f 1)"
  resource_compiler_environment
  log "build the setup wizard of $target"
  # The wizard has its own Cargo.lock and is not a member of the workspace, so
  # the build directory is named here.
  CARGO_TARGET_DIR="$target_dir" PATH="$root/packaging/release/msvc-clang:$PATH" \
    NURT_SETUP_PAYLOAD="$archive" NURT_SETUP_PAYLOAD_SHA256="$sum" \
    cargo xwin build --release --locked --manifest-path "$setup_manifest" --target "$target"
  built="$target_dir/$target/release/nurt-setup.exe"
  [ -f "$built" ] || die "missing $built"
  cp "$built" "$output/nurt-$version-$target-setup.exe"
  if [ "$codesign" -eq 1 ]; then sign_file "$output/nurt-$version-$target-setup.exe"; fi
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
  case "$target" in *-windows-*) binary="$binary.exe" ;; esac
  [ -f "$binary" ] || die "missing $binary"
  case "$target" in
    *-windows-*)
      if [ "$codesign" -eq 1 ]; then
        # The archive is packed from a signed copy, so that no file of the
        # output directory changes after its sum is computed.
        mkdir -p "$work/signed/$target"
        cp "$binary" "$work/signed/$target/nurt.exe"
        sign_file "$work/signed/$target/nurt.exe"
        binary="$work/signed/$target/nurt.exe"
      fi ;;
  esac
  library="$(library_file "$target")"
  [ -f "$library" ] || die "missing $library"
  case "$target" in
    *-linux-musl)
      mkdir -p "$work/library/$target"
      shrink_static_library "$library" "$work/library/$target/libnurt_ffi.a"
      library="$work/library/$target/libnurt_ffi.a"
      ;;
  esac
  library_args=(--library "$library")
  man_args=(--man "$work/man")
  case "$target" in *-windows-*) man_args=() ;; esac
  log "pack $target"
  python3 -I "$root/packaging/release/pack.py" --root "$root" --version "$version" \
    --target "$target" --binary "$binary" "${library_args[@]}" "${man_args[@]}" \
    --completions "$work/completions" --site "$work/site" --output "$output"
  case "$target" in
    *-windows-*) if [ "$setup" -eq 1 ]; then build_setup "$target"; fi ;;
  esac
done

log "checksums"
(
  cd "$output"
  files=()
  for file in nurt-*.tar.gz nurt-*.zip nurt-*-setup.exe; do
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

# Runs the executable of the program archive of a target that the host runs.
# Argument: target.
run_executable() {
  local target="$1" run="$work/run/$1"
  mkdir -p "$run/account"
  tar -xzf "$output/nurt-$version-$target.tar.gz" -C "$run"
  local nurt="$run/nurt-$version-$target/nurt"
  [ -x "$nurt" ] || die "the archive has no executable $nurt"
  (
    cd "$run"
    export HOME="$run/account" XDG_STATE_HOME="$run/account/state" XDG_CONFIG_HOME="$run/account/config"
    export LD_LIBRARY_PATH="$run/nurt-$version-$target"
    "$nurt" version
    "$nurt" doctor
  )
}

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

# Only the target of the host is run: the executable of its archive, and the C
# example against its library archive. The static musl library of the host
# architecture is checked on an x86_64 Linux host as well, because the program
# linked with it is static.
host_target_built=0
for target in "${targets[@]}"; do
  if [ "$target" = "$host" ]; then
    host_target_built=1
    log "run the executable of the archive of $target"
    run_executable "$target"
    log "build and run the C example against the library of $target"
    check_c_example "$target" cc ""
  elif [ "$target" = "$LINUX_MUSL" ] && [ "$host" = "$LINUX_GNU" ]; then
    log "build and run the C example against the static library of $target"
    check_c_example "$target" "${CC_x86_64_unknown_linux_musl:-$root/packaging/release/zig-cc-musl.sh}" "-static"
  fi
done
if [ "$host_target_built" -eq 0 ]; then
  log "the host $host is not a target of this release: no executable was run"
fi

# The names in the output directory are the artifact names of this script.
# shellcheck disable=SC2012
log "done: $(ls "$output" | tr '\n' ' ')"

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
