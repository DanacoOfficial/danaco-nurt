# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B37: diff, counting, sizes, hashes and conversions.
diff /srv/a.conf /srv/b.conf
sort /srv/words.txt | uniq -c
du -sh /srv/app
stat /srv/app/data.db
file /srv/app/bin/run
sha256sum /srv/app.tgz
dos2unix /srv/a.txt
iconv -f latin1 -t utf-8 /srv/a.txt
