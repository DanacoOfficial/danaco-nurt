# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B22: two effects joined with && and with ;.
mkdir -p /srv/out && cp /srv/a.txt /srv/out
rm /srv/a.txt; rm /srv/b.txt
