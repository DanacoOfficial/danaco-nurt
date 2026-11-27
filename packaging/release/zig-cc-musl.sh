#!/bin/sh
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           packaging/release/zig-cc-musl.sh
# Responsibility:   C compiler of the musl target x86_64-unknown-linux-musl: runs "zig cc" and drops.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

#
# C compiler of the musl target x86_64-unknown-linux-musl: runs "zig cc" and drops
# the clang-style option --target=<triple> that the cc crate adds, because it
# conflicts with the option -target of zig. Set CC_x86_64_unknown_linux_musl to
# the path of this script; the linker stays the default cc. The executable of
# zig is taken from ZIG and is "zig" when ZIG is unset.
for argument in "$@"; do
    shift
    case $argument in
        --target=*) ;;
        *) set -- "$@" "$argument" ;;
    esac
done
exec "${ZIG:-zig}" cc -target x86_64-linux-musl "$@"

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
