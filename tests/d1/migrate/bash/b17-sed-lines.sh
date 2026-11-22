# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B17: sed line ranges and deletions.
sed -n '10,20p' /var/log/app.log
sed -i '/^#/d' /etc/app.conf
sed -i '3d' /srv/list.txt
