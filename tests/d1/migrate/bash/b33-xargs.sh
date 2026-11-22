# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B33: xargs and find -exec.
find /srv -name "*.bak" | xargs rm
find /srv/app -name "*.sh" -exec chmod 755 {} +
find /srv/app -name "*.go" | xargs gofmt -l
