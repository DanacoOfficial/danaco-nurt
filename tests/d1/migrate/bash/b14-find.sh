# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B14: find with name, type, time, size, depth and delete.
find /srv/app -name "*.log" -type f -mtime -7
find /tmp -name "*.tmp" -delete
find /srv -maxdepth 2 -type d -name build
find /srv -iname "readme*" -size +1M
