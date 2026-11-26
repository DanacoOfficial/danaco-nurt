#!/bin/sh
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# C compiler and linker of the target aarch64-unknown-linux-gnu: runs "zig cc".
# Drops the clang-style option --target=<triple> that the cc crate adds and the
# linker options -Wl,--fix-cortex-a53-843419 (rejected by zig) and -Wl,-O1
# (deprecated, zig warns) that rustc adds. The executable of zig is taken from
# ZIG and is "zig" when ZIG is unset.
for argument in "$@"; do
    shift
    case $argument in
        --target=*) ;;
        -Wl,--fix-cortex-a53-843419) ;;
        -Wl,-O1) ;;
        *) set -- "$@" "$argument" ;;
    esac
done
exec "${ZIG:-zig}" cc -target aarch64-linux-gnu.2.31 "$@"
