# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B21: test and brackets guarding a command.
test -f /srv/app/.ready && rm /srv/app/.ready
[ -d /srv/app/out ] && rm -rf /srv/app/out
