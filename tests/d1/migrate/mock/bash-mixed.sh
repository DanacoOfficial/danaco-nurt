#!/bin/bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Rebuilds the application and archives the output.
set -e

rm -rf /srv/app/build
cd /srv/app && go test ./...
SOURCES=$(find /srv/app/src -name '*.go')
grep -rn "oldName(" $SOURCES

mkdir -p /srv/app/out; cp -r /srv/app/static /srv/app/out
ls /srv/app/out || echo "missing output"

for f in /srv/app/out/*.tmp; do
  rm "$f"
done

cleanup() { rm -rf /tmp/app; }
eval "$EXTRA_STEPS"
source /etc/app/env.sh

cat > /srv/app/out/VERSION <<EOF
1.4.2
EOF
sudo systemctl restart app
cd /srv/app
tar -czf /srv/backup/app.tgz /srv/app/out &
if [ -f /srv/app/out/VERSION ]; then echo ok; fi
rm "/srv/app/out/unterminated
