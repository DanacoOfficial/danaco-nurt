#!/bin/bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Scenario run by "devcontainer features test" with the default options.

set -e

# shellcheck source=/dev/null
source dev-container-features-test-lib

check "nurt is on the PATH" bash -c "command -v nurt"
check "nurt prints its version" bash -c "nurt version | grep -F 'Danaco Nurt'"

reportResults
