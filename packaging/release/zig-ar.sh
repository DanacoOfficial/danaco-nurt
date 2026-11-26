#!/bin/sh
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Archiver of the cross-built Linux targets: runs "zig ar". The executable of zig
# is taken from ZIG and is "zig" when ZIG is unset.
exec "${ZIG:-zig}" ar "$@"
