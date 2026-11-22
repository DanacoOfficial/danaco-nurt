# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B39: loops, conditions, functions, case and here-documents.
for f in /srv/logs/*.log; do gzip "$f"; done
while true; do sleep 5; done
if [ -f /srv/app/.ready ]; then echo ready; fi
case "$1" in start) echo start;; stop) echo stop;; esac
deploy() { rsync -a /srv/app/ /srv/www/; }
cat > /srv/app/motd <<EOF2
Maintenance at 22:00.
EOF2
