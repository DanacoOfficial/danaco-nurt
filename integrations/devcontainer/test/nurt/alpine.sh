#!/bin/sh
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           integrations/devcontainer/test/nurt/alpine.sh
# Responsibility:   Scenario "alpine": the executable runs on a musl based image.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.2, 2026-11-28
# License:          see the LICENSE file or https://www.danaco-group.pl

# Scenario "alpine": the executable runs on a musl based image.

# The image has no bash, and the test library of the dev container CLI is a bash
# script, so the scenario runs its check itself; the exit code is the result.

set -e

version="$(nurt version)"
printf '%s\n' "$version"
printf '%s\n' "$version" | grep -E '^nurt [0-9]' >/dev/null

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
