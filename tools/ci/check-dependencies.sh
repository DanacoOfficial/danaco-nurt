#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Dependency check of the release binaries.
#
# A release binary runs on a clean system, so it may depend only on the system
# libraries listed below. The script reads the dynamic section of every file
# given and fails when the file names a shared library outside the list.
#
#   ELF (Linux)   readelf -d, entries of type NEEDED. A statically linked file
#                 has no dynamic section and passes.
#   PE (Windows)  the list of the permitted DLLs is kept in
#                 WINDOWS_ALLOWED_DLLS and read by check_pe; the check itself
#                 is not implemented yet.
#
# Usage:
#   check-dependencies.sh [--format elf|pe] FILE...
#
# The format is taken from the first bytes of each file when --format is
# omitted.
#
# Environment:
#   READELF   program that prints the dynamic section, default readelf
#
# Exit status: 0 every file passes, 1 a file has a dependency outside the list
# or cannot be read, 2 invalid arguments or a missing program.

set -euo pipefail

# Shared libraries a Linux binary may name. The C library stays dynamic; every
# other dependency is linked into the file. The names cover x86-64 and aarch64.
ELF_ALLOWED_LIBRARIES=(
    "libc.so.6"
    "libm.so.6"
    "libgcc_s.so.1"
    "ld-linux-x86-64.so.2"
    "ld-linux-aarch64.so.1"
    "linux-vdso.so.1"
)

# DLLs a Windows binary may name. Filled in by the Windows check.
WINDOWS_ALLOWED_DLLS=()

readelf_program=${READELF:-readelf}

usage() {
    echo "usage: check-dependencies.sh [--format elf|pe] FILE..." >&2
    exit 2
}

fail_usage() {
    echo "check-dependencies.sh: $*" >&2
    exit 2
}

detect_format() {
    local file=$1 magic
    magic=$(LC_ALL=C head -c 4 -- "$file" | od -An -tx1 | tr -d ' \n')
    case $magic in
        7f454c46) echo elf ;;
        4d5a*) echo pe ;;
        *) echo unknown ;;
    esac
}

is_allowed_library() {
    local name=$1 allowed
    for allowed in "${ELF_ALLOWED_LIBRARIES[@]}"; do
        [ "$name" = "$allowed" ] && return 0
    done
    return 1
}

# Prints the NEEDED entries of an ELF file, one name per line.
elf_needed() {
    local file=$1 dynamic
    if ! dynamic=$("$readelf_program" -d -- "$file" 2>&1); then
        echo "check-dependencies.sh: $readelf_program failed on $file: $dynamic" >&2
        return 1
    fi
    printf '%s\n' "$dynamic" | sed -n 's/.*(NEEDED)[[:space:]]*Shared library: \[\(.*\)\].*/\1/p'
}

check_elf() {
    local file=$1 needed name result=0
    needed=$(elf_needed "$file") || return 1
    while IFS= read -r name; do
        [ -n "$name" ] || continue
        if is_allowed_library "$name"; then
            continue
        fi
        echo "check-dependencies.sh: $file depends on $name, which is not in the list of permitted libraries" >&2
        result=1
    done <<<"$needed"
    return $result
}

# Windows check. A PE file names its DLLs in the import table; the check reads
# the table and compares every name, case-insensitively, with
# WINDOWS_ALLOWED_DLLS.
check_pe() {
    local file=$1
    echo "check-dependencies.sh: the PE check of $file is not implemented" >&2
    return 1
}

format=
files=()
while [ $# -gt 0 ]; do
    case $1 in
        --format)
            [ $# -ge 2 ] || usage
            format=$2
            shift 2
            ;;
        -h | --help) usage ;;
        --) shift; files+=("$@"); break ;;
        -*) fail_usage "unknown option: $1" ;;
        *) files+=("$1"); shift ;;
    esac
done
[ ${#files[@]} -gt 0 ] || usage
case $format in
    '' | elf | pe) ;;
    *) fail_usage "unknown format: $format" ;;
esac
command -v "$readelf_program" >/dev/null 2>&1 || fail_usage "$readelf_program is not installed"

status=0
for file in "${files[@]}"; do
    if [ ! -f "$file" ]; then
        echo "check-dependencies.sh: $file is not a file" >&2
        status=1
        continue
    fi
    kind=${format:-$(detect_format "$file")}
    result=0
    case $kind in
        elf) check_elf "$file" || result=1 ;;
        pe) check_pe "$file" || result=1 ;;
        *)
            echo "check-dependencies.sh: $file is neither an ELF nor a PE file" >&2
            result=1
            ;;
    esac
    if [ $result -eq 0 ]; then
        echo "ok: $file"
    else
        status=1
    fi
done
exit $status
