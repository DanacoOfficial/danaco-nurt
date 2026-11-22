# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B40: schedules, services, packages, dates and pauses.
crontab -e
systemctl restart nginx
systemctl enable app
service cron restart
apt install -y jq
brew install jq
date
sleep 5
