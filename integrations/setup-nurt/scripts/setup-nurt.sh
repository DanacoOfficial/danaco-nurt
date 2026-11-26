#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Installs a Danaco Nurt release into the runner tool cache.
#
# Inputs (environment): INPUT_VERSION (default "latest"), INPUT_VERIFY
# (default "true"). Outputs are written to GITHUB_OUTPUT and the install
# directory is appended to GITHUB_PATH when those variables are set.
# NURT_RELEASES_URL and NURT_DOWNLOAD_BASE override the release locations.

set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib.sh
source "$script_dir/lib.sh"

to_native_path() {
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -w "$1"
  else
    printf '%s\n' "$1"
  fi
}

to_posix_path() {
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -u "$1"
  else
    printf '%s\n' "$1"
  fi
}

WORK_DIR=''
trap 'if [[ -n $WORK_DIR ]]; then rm -rf "$WORK_DIR"; fi' EXIT

main() {
  local spec=${INPUT_VERSION:-latest}
  local verify=${INPUT_VERIFY:-true}
  local os=${RUNNER_OS:-$(uname -s)}
  local arch=${RUNNER_ARCH:-$(uname -m)}

  case $verify in
    true | false) ;;
    *) die "input 'verify' must be 'true' or 'false'" ;;
  esac

  local target version tool_arch binary
  target=$(release_target "$os" "$arch")
  tool_arch=$(cache_arch "$arch")
  version=$(resolve_version "$spec")
  binary=nurt
  case $target in
    *windows*) binary=nurt.exe ;;
  esac

  local cache_root
  cache_root=$(to_posix_path "${RUNNER_TOOL_CACHE:-${TMPDIR:-/tmp}/nurt-toolcache}")
  local version_dir="$cache_root/nurt/$version"
  local install_dir="$version_dir/$tool_arch"
  local marker="$version_dir/$tool_arch.complete"

  if [[ -f $marker && -x $install_dir/$binary ]]; then
    printf 'using cached Danaco Nurt %s (%s)\n' "$version" "$tool_arch"
  else
    install_release "$version" "$target" "$verify" "$install_dir" "$binary"
    : >"$marker"
  fi

  publish_outputs "$install_dir" "$binary" "$version"
}

install_release() {
  local version=$1 target=$2 verify=$3 install_dir=$4 binary=$5
  local base name url work
  base=$(release_base)
  name=$(archive_name "$version" "$target")
  url="$base/v$version/$name"
  work=$(mktemp -d)
  WORK_DIR=$work

  printf 'downloading %s\n' "$url"
  fetch "$url" "$work/$name" || die "cannot download $name for version $version"

  if [[ $verify == true ]]; then
    fetch "$base/v$version/checksums.txt" "$work/checksums.txt" ||
      die "cannot download checksums.txt for version $version; set 'verify' to false to skip verification"
    verify_checksum "$work/$name" "$name" "$work/checksums.txt"
    printf 'verified SHA-256 of %s\n' "$name"
  fi

  mkdir -p "$work/extract"
  extract_archive "$work/$name" "$work/extract"
  local root=$work/extract
  [[ -f $root/$binary ]] || root=$root/nurt-$version-$target
  [[ -f $root/$binary ]] || die "$name does not contain $binary"

  rm -rf "$install_dir"
  mkdir -p "$install_dir"
  cp -R "$root/." "$install_dir/"
  chmod +x "$install_dir/$binary"
}

publish_outputs() {
  local install_dir=$1 binary=$2 version=$3
  local out_path
  out_path=$(to_native_path "$install_dir/$binary")
  if [[ -n ${GITHUB_PATH:-} ]]; then
    to_native_path "$install_dir" >>"$GITHUB_PATH"
  fi
  if [[ -n ${GITHUB_OUTPUT:-} ]]; then
    {
      printf 'path=%s\n' "$out_path"
      printf 'version=%s\n' "$version"
    } >>"$GITHUB_OUTPUT"
  else
    printf 'path=%s\nversion=%s\n' "$out_path" "$version"
  fi
}

main "$@"
