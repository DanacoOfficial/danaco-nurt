# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B04: cp, recursive and archive copies.
cp /etc/app.conf /srv/backup/app.conf
cp -r /srv/app/static /srv/www
cp -a /srv/data /mnt/backup
