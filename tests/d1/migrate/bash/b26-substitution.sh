# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B26: command substitution bound to a variable and used later.
LOGS=$(find /var/log/app -name "*.log")
rm $LOGS
COUNT=`wc -l < /srv/a.txt`
echo $COUNT
