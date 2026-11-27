#!/bin/sh
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Installs the nurt executable of a Danaco Nurt release.
#
#   curl -fsSL https://nurt.danaco-group.pl/install.sh | sh
#   sh install.sh [--version VERSION] [--bin-dir DIR] [--public-key FILE|URL]
#
# The script detects the operating system and the processor architecture,
# downloads the release archive, verifies its SHA-256 digest against the
# checksums.txt file of the release and installs the executable. The default
# installation directory is $HOME/.local/bin; no privileges are needed. For a
# system-wide installation run the script as root with --bin-dir /usr/local/bin.
#
# On Linux the script takes the static musl archive when the release has one
# and the glibc archive otherwise; a machine with musl takes only the musl one.
#
# The checksums.txt file is verified against its detached signature
# (checksums.txt.asc) when a public key is given and gpg is installed.
#
# Environment variables:
#   NURT_VERSION       release version to install (default: the latest release)
#   NURT_INSTALL_DIR   installation directory
#   NURT_PUBLIC_KEY    file or URL of the ASCII-armored public signing key
#   NURT_RELEASES_URL  release location; URL, directory or file:// URL with
#                      the layout of GitHub releases (default:
#                      https://github.com/DanacoOfficial/danaco-nurt/releases)

set -eu

releases_url=${NURT_RELEASES_URL:-https://github.com/DanacoOfficial/danaco-nurt/releases}
version=${NURT_VERSION:-}
bin_dir=${NURT_INSTALL_DIR:-}
public_key=${NURT_PUBLIC_KEY:-}

say() {
    printf 'nurt-install: %s\n' "$*" >&2
}

fail() {
    say "$*"
    exit 1
}

usage() {
    say "usage: install.sh [--version VERSION] [--bin-dir DIR] [--public-key FILE|URL]"
    exit 2
}

have() {
    command -v "$1" >/dev/null 2>&1
}

# fetch LOCATION DESTINATION
fetch() {
    case $1 in
        http://* | https://*)
            if have curl; then
                curl -fsSL --retry 3 -o "$2" "$1"
            elif have wget; then
                wget -q -O "$2" "$1"
            else
                fail "curl or wget is required to download $1"
            fi
            ;;
        file://*) cp "${1#file://}" "$2" ;;
        *) cp "$1" "$2" ;;
    esac
}

sha256_of() {
    if have sha256sum; then
        sha256sum "$1" | awk '{ print $1 }'
    elif have shasum; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    elif have openssl; then
        openssl dgst -sha256 -r "$1" | awk '{ print $1 }'
    else
        fail "sha256sum, shasum or openssl is required to verify the download"
    fi
}

# digest_in CHECKSUMS NAME
digest_in() {
    awk -v name="$2" '
        { file = $2; sub(/^\*/, "", file) }
        file == name { print tolower($1); exit }
    ' "$1"
}

# detect_targets sets targets to the release targets of this machine, the
# preferred one first. A Linux machine with glibc takes the static musl build
# when the release has one and the glibc build otherwise; a machine with musl
# takes only the musl build.
detect_targets() {
    system=$(uname -s)
    machine=$(uname -m)
    case $machine in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) fail "unsupported architecture: $machine" ;;
    esac
    case $system in
        Linux)
            if have ldd && ldd --version 2>&1 | grep -qi musl; then
                targets=$arch-unknown-linux-musl
            else
                targets="$arch-unknown-linux-musl $arch-unknown-linux-gnu"
            fi
            ;;
        Darwin) targets=$arch-apple-darwin ;;
        *) fail "unsupported operating system: $system" ;;
    esac
}

verify_signature() {
    checksums=$1
    if [ -z "$public_key" ]; then
        say "signature not verified: no public key given"
        return 0
    fi
    have gpg || fail "gpg is required to verify the signature with the given public key"
    fetch "$releases_url/download/v$version/checksums.txt.asc" "$work/checksums.txt.asc" ||
        fail "checksums.txt.asc is not available for version $version"
    fetch "$public_key" "$work/public-key.asc" || fail "cannot read the public key $public_key"
    mkdir -m 700 "$work/gnupg"
    GNUPGHOME=$work/gnupg gpg --batch --quiet --import "$work/public-key.asc" 2>/dev/null ||
        fail "the public key cannot be imported"
    GNUPGHOME=$work/gnupg gpg --batch --quiet --verify "$work/checksums.txt.asc" "$checksums" 2>/dev/null ||
        fail "signature of checksums.txt is not valid"
}

while [ $# -gt 0 ]; do
    case $1 in
        --version | --bin-dir | --public-key)
            [ $# -ge 2 ] || usage
            case $1 in
                --version) version=$2 ;;
                --bin-dir) bin_dir=$2 ;;
                --public-key) public_key=$2 ;;
            esac
            shift 2
            ;;
        -h | --help) usage ;;
        *) usage ;;
    esac
done

version=${version#v}
case $version in
    *[!0-9A-Za-z.+-]*) fail "invalid version: $version" ;;
esac

if [ -z "$bin_dir" ]; then
    [ -n "${HOME:-}" ] || fail "HOME is not set; use --bin-dir"
    bin_dir=$HOME/.local/bin
fi

detect_targets

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
trap 'exit 1' INT TERM

if [ -z "$version" ]; then
    fetch "$releases_url/latest/download/checksums.txt" "$work/latest.txt" ||
        fail "cannot read the checksums of the latest release"
    for candidate in $targets; do
        version=$(sed -n "s/^[0-9A-Fa-f]\{64\}  *\**nurt-\(.*\)-$candidate\.tar\.gz\$/\1/p" "$work/latest.txt" | head -n 1)
        [ -n "$version" ] && break
    done
    [ -n "$version" ] || fail "the latest release has no archive for $targets"
    case $version in
        *[!0-9A-Za-z.+-]*) fail "invalid version in the latest release: $version" ;;
    esac
fi

download_url=$releases_url/download/v$version

fetch "$download_url/checksums.txt" "$work/checksums.txt" ||
    fail "cannot read the checksums of version $version"
verify_signature "$work/checksums.txt"

target=
expected=
for candidate in $targets; do
    digest=$(digest_in "$work/checksums.txt" "nurt-$version-$candidate.tar.gz")
    if [ -n "$digest" ]; then
        target=$candidate
        expected=$digest
        break
    fi
done
[ -n "$expected" ] || fail "version $version has no archive for $targets"
archive=nurt-$version-$target.tar.gz

fetch "$download_url/$archive" "$work/$archive" || fail "cannot download $archive"
actual=$(sha256_of "$work/$archive")
[ "$actual" = "$expected" ] || fail "SHA-256 mismatch for $archive: expected $expected, found $actual"

mkdir "$work/extract"
tar -xzf "$work/$archive" -C "$work/extract"
binary=$work/extract/nurt-$version-$target/nurt
[ -f "$binary" ] || fail "$archive does not contain nurt"

mkdir -p "$bin_dir"
cp "$binary" "$bin_dir/.nurt.$$"
chmod 755 "$bin_dir/.nurt.$$"
mv -f "$bin_dir/.nurt.$$" "$bin_dir/nurt"

say "installed nurt $version to $bin_dir/nurt"
case ":${PATH:-}:" in
    *":$bin_dir:"*) ;;
    *) say "$bin_dir is not in PATH" ;;
esac
