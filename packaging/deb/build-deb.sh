#!/bin/sh
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Builds the Debian packages nurt, libnurt1 and libnurt-dev from a release
# archive of the glibc Linux target.
#
# Usage:
#   build-deb.sh --artifact PATH --version VERSION --arch amd64|arm64 --output DIR
#
# PATH is the release archive (.tar.gz) or the directory with its contents.
# SOURCE_DATE_EPOCH must be set: it fixes the file times and the changelog
# date, so that the same input produces byte-identical packages.

set -eu
umask 022

here=$(cd "$(dirname "$0")" && pwd)

maintainer="Danaco Official <support@danaco-group.pl>"
homepage="https://nurt.danaco-group.pl"
product_source=${NURT_PRODUCT_SOURCE:-$here/../../crates/nurt-contracts/src/product.rs}

read_tagline() {
    [ -f "$product_source" ] || fail "product texts not found: $product_source"
    tr '\n' ' ' <"$product_source" |
        sed -n 's/.*pub const TAGLINE: &str =[[:space:]]*"\([^"]*\)".*/\1/p'
}

usage() {
    echo "usage: build-deb.sh --artifact PATH [--lib-artifact PATH] --version VERSION --arch amd64|arm64 --output DIR" >&2
    exit 2
}

fail() {
    echo "build-deb.sh: $*" >&2
    exit 1
}

summary=$(read_tagline)
[ -n "$summary" ] || fail "TAGLINE not found in $product_source"

artifact=
version=
arch=
output=
lib_artifact=
while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || usage
    case $1 in
        --artifact) artifact=$2 ;;
        --lib-artifact) lib_artifact=$2 ;;
        --version) version=$2 ;;
        --arch) arch=$2 ;;
        --output) output=$2 ;;
        *) usage ;;
    esac
    shift 2
done
[ -n "$artifact" ] && [ -n "$version" ] && [ -n "$arch" ] && [ -n "$output" ] || usage
[ -n "${SOURCE_DATE_EPOCH:-}" ] || fail "SOURCE_DATE_EPOCH is not set"
command -v dpkg-deb >/dev/null 2>&1 || fail "dpkg-deb is not installed"

case $arch in
    amd64) multiarch=x86_64-linux-gnu ;;
    arm64) multiarch=aarch64-linux-gnu ;;
    *) fail "unsupported architecture: $arch" ;;
esac

case $version in
    *-*) deb_version=$(printf '%s' "$version" | sed 's/-/~/') ;;
    *) deb_version=$version ;;
esac

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
trap 'exit 1' INT TERM

# Prints the directory with the contents of an archive, or the directory itself.
unpack() {
    source_path=$1
    slot=$2
    if [ -d "$source_path" ]; then
        printf '%s\n' "$source_path"
        return
    fi
    [ -f "$source_path" ] || fail "artifact not found: $source_path"
    mkdir "$work/$slot"
    tar -xzf "$source_path" -C "$work/$slot"
    unpacked=$work/$slot
    entries=$(ls -A "$unpacked")
    if [ "$(printf '%s\n' "$entries" | wc -l)" -eq 1 ] && [ -d "$unpacked/$entries" ]; then
        unpacked=$unpacked/$entries
    fi
    printf '%s\n' "$unpacked"
}

root=$(unpack "$artifact" src) || exit 1
# The library, the header and the pkg-config file are in the library archive
# nurt-lib-<version>-<target>; without --lib-artifact they are read from the
# program archive.
if [ -n "$lib_artifact" ]; then
    lib_root=$(unpack "$lib_artifact" lib) || exit 1
else
    lib_root=$root
fi

[ -f "$root/nurt" ] || fail "artifact lacks nurt"
[ -f "$root/NOTICE" ] || fail "artifact lacks NOTICE"
for required in libnurt.so.1 nurt.h nurt.pc; do
    [ -f "$lib_root/$required" ] || fail "artifact lacks $required"
done
for variable in prefix libdir includedir; do
    grep -q "^$variable=" "$lib_root/nurt.pc" || fail "nurt.pc does not define $variable"
done

date_rfc=$(date -u -d "@$SOURCE_DATE_EPOCH" -R)

# Installs a file or a directory tree with fixed modes.
install_tree() {
    source_dir=$1
    target_dir=$2
    mkdir -p "$target_dir"
    cp -R "$source_dir/." "$target_dir/"
    find "$target_dir" -type d -exec chmod 755 {} +
    find "$target_dir" -type f -exec chmod 644 {} +
}

write_docs() {
    package=$1
    docdir=$2/usr/share/doc/$package
    mkdir -p "$docdir"
    install -m 644 "$here/copyright" "$docdir/copyright"
    {
        printf '%s (%s) stable; urgency=medium\n\n' nurt "$deb_version"
        printf '  * Release %s. Changes are listed in CHANGELOG.md of the source tree.\n\n' "$version"
        printf ' -- %s  %s\n' "$maintainer" "$date_rfc"
    } | gzip -9n >"$docdir/changelog.gz"
    chmod 644 "$docdir/changelog.gz"
}

write_control() {
    package=$1
    dir=$2
    shift 2
    mkdir -p "$dir/DEBIAN"
    size=$(du -sk --exclude=DEBIAN "$dir" | cut -f1)
    {
        printf 'Package: %s\n' "$package"
        printf 'Source: nurt\n'
        printf 'Version: %s\n' "$deb_version"
        printf 'Architecture: %s\n' "$arch"
        printf 'Maintainer: %s\n' "$maintainer"
        printf 'Installed-Size: %s\n' "$size"
        printf '%s\n' "$@"
        printf 'Homepage: %s\n' "$homepage"
    } >"$dir/DEBIAN/control"
    chmod 644 "$dir/DEBIAN/control"
}

build_package() {
    package=$1
    dir=$2
    find "$dir" -depth -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
    dpkg-deb --root-owner-group -Zxz --build "$dir" \
        "$output/${package}_${deb_version}_${arch}.deb" >/dev/null
}

mkdir -p "$output"

nurt_dir=$work/pkg/nurt
mkdir -p "$nurt_dir/usr/bin"
install -m 755 "$root/nurt" "$nurt_dir/usr/bin/nurt"
if [ -d "$root/man" ]; then
    install_tree "$root/man" "$nurt_dir/usr/share/man"
    find "$nurt_dir/usr/share/man" -type f -name '*.[0-9]' -exec gzip -9n {} +
fi
if [ -f "$root/completions/nurt.bash" ]; then
    mkdir -p "$nurt_dir/usr/share/bash-completion/completions"
    install -m 644 "$root/completions/nurt.bash" "$nurt_dir/usr/share/bash-completion/completions/nurt"
fi
if [ -f "$root/completions/_nurt" ]; then
    mkdir -p "$nurt_dir/usr/share/zsh/vendor-completions"
    install -m 644 "$root/completions/_nurt" "$nurt_dir/usr/share/zsh/vendor-completions/_nurt"
fi
if [ -f "$root/completions/nurt.fish" ]; then
    mkdir -p "$nurt_dir/usr/share/fish/vendor_completions.d"
    install -m 644 "$root/completions/nurt.fish" "$nurt_dir/usr/share/fish/vendor_completions.d/nurt.fish"
fi
if [ -d "$root/service" ]; then
    install_tree "$root/service" "$nurt_dir/usr/share/nurt/service"
fi
write_docs nurt "$nurt_dir"
install -m 644 "$root/NOTICE" "$nurt_dir/usr/share/doc/nurt/NOTICE"
write_control nurt "$nurt_dir" \
    "Section: utils" \
    "Priority: optional" \
    "Multi-Arch: foreign" \
    "Depends: libc6 (>= 2.17), libgcc-s1" \
    "Description: $summary" \
    " Danaco Nurt is a command shell with its own command language. The" \
    " program is a single executable."
build_package nurt "$nurt_dir"

lib_dir=$work/pkg/libnurt1
mkdir -p "$lib_dir/usr/lib/$multiarch"
install -m 644 "$lib_root/libnurt.so.1" "$lib_dir/usr/lib/$multiarch/libnurt.so.1"
write_docs libnurt1 "$lib_dir"
write_control libnurt1 "$lib_dir" \
    "Section: libs" \
    "Priority: optional" \
    "Multi-Arch: same" \
    "Depends: libc6 (>= 2.17)" \
    "Description: C interface library of Danaco Nurt" \
    " The shared library libnurt provides the C interface of the Danaco Nurt" \
    " command shell, declared in nurt.h."
printf 'libnurt 1 libnurt1 (>= %s)\n' "$deb_version" >"$lib_dir/DEBIAN/shlibs"
printf 'activate-noawait ldconfig\n' >"$lib_dir/DEBIAN/triggers"
chmod 644 "$lib_dir/DEBIAN/shlibs" "$lib_dir/DEBIAN/triggers"
build_package libnurt1 "$lib_dir"

dev_dir=$work/pkg/libnurt-dev
mkdir -p "$dev_dir/usr/include" "$dev_dir/usr/lib/$multiarch/pkgconfig"
install -m 644 "$lib_root/nurt.h" "$dev_dir/usr/include/nurt.h"
ln -s libnurt.so.1 "$dev_dir/usr/lib/$multiarch/libnurt.so"
sed -e 's|^prefix=.*|prefix=/usr|' \
    -e "s|^libdir=.*|libdir=\${prefix}/lib/$multiarch|" \
    -e "s|^includedir=.*|includedir=\${prefix}/include|" \
    "$lib_root/nurt.pc" >"$dev_dir/usr/lib/$multiarch/pkgconfig/nurt.pc"
chmod 644 "$dev_dir/usr/lib/$multiarch/pkgconfig/nurt.pc"
write_docs libnurt-dev "$dev_dir"
write_control libnurt-dev "$dev_dir" \
    "Section: libdevel" \
    "Priority: optional" \
    "Multi-Arch: same" \
    "Depends: libnurt1 (= $deb_version)" \
    "Description: C interface of Danaco Nurt, development files" \
    " The package provides the header nurt.h, the linker symlink and the" \
    " pkg-config file of the libnurt C interface."
build_package libnurt-dev "$dev_dir"

printf '%s\n' "$output/nurt_${deb_version}_${arch}.deb" \
    "$output/libnurt1_${deb_version}_${arch}.deb" \
    "$output/libnurt-dev_${deb_version}_${arch}.deb"
