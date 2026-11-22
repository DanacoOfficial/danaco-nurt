# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B18: awk field printing and conditions.
awk '{print $1}' /var/log/nginx/access.log
awk -F: '{print $1, $7}' /etc/passwd
awk '$3 > 100 {print $1}' /srv/data.txt
