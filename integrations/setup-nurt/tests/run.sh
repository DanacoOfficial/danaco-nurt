#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Runs every test file in this directory and fails if any of them fails.

set -uo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

status=0
for test_file in "$here"/test_*.sh; do
  printf '== %s\n' "$(basename "$test_file")"
  bash "$test_file" || status=1
done
exit "$status"
