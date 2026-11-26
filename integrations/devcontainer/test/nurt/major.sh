#!/bin/bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Scenario "major": the version option is a major prefix.

set -e

# shellcheck source=/dev/null
source dev-container-features-test-lib

check "installed version has the requested major" bash -c "nurt version | grep -E '^nurt 1\.'"

reportResults
