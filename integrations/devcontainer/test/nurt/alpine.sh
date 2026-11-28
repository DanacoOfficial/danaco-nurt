#!/bin/sh
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           integrations/devcontainer/test/nurt/alpine.sh
# Responsibility:   Scenario "alpine": the executable runs on a musl based image.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

# Scenario "alpine": the executable runs on a musl based image.

set -e

# shellcheck source=/dev/null
. dev-container-features-test-lib

check "nurt prints its version" sh -c "nurt version | grep -E '^nurt [0-9]'"

reportResults

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
