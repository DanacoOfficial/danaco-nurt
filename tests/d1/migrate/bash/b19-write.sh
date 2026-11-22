# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B19: echo, printf and tee written to files.
echo "ready" > /srv/app/status
echo "line" >> /srv/app/log.txt
printf '%s\n' done > /srv/app/done
echo -n "x" > /srv/app/flag
date | tee /srv/app/started
