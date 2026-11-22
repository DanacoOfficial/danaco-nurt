# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# B31: environment variables for one command.
DEBUG=1 /srv/app/bin/run
env LANG=C sort /srv/a.txt
export GOFLAGS=-mod=vendor; go build -C /srv/app
