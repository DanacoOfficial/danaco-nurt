# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B15: grep with recursion, case, inversion, counts and file lists.
grep -rn "oldName(" /srv/app/src
grep -i error /var/log/app.log
grep -rl --include='*.go' "func main" /srv/app
grep -c "GET /" /var/log/nginx/access.log
grep -vE '^#|^$' /etc/app.conf
grep -w -C 2 timeout /etc/app.conf
