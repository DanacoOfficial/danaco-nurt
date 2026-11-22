# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B34: tar, zip and gzip.
tar -xzf /tmp/app.tgz -C /srv/app
tar -czf /srv/backup/app.tgz /srv/app
unzip /tmp/data.zip -d /srv/data
zip -r /srv/out.zip /srv/out
gzip /var/log/app.log
gunzip /srv/data.csv.gz
