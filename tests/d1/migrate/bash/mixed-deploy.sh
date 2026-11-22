#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Mixed: variables, loops, functions, evaluation, here-documents and sudo.
RELEASE=/srv/releases/1.4.2
mkdir -p $RELEASE && tar -xzf /tmp/app.tgz -C $RELEASE
ln -sfn $RELEASE /srv/app/current
for f in /srv/app/current/conf/*.dist; do cp "$f" "${f%.dist}"; done
notify() { curl -s -X POST -d "$1" https://hooks.example.com/deploy; }
eval "$POST_DEPLOY"
cat > /srv/app/current/VERSION <<EOF2
1.4.2
EOF2
cd /srv/app/current
sudo systemctl restart app || journalctl -u app -n 50
