#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           integrations/vscode/scripts/test-copy-grammar.sh
# Responsibility:   Tests copy-grammar.sh with grammar files in a temporary directory.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

# Tests copy-grammar.sh with grammar files in a temporary directory.

set -uo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
script="$here/copy-grammar.sh"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

failures=0

expect() {
  local description=$1
  shift
  if "$@"; then
    printf 'ok   %s\n' "$description"
  else
    printf 'FAIL %s\n' "$description"
    failures=$((failures + 1))
  fi
}

copy() {
  env NURT_GRAMMAR_SOURCE="$1" NURT_GRAMMAR_TARGET="$2" bash "$script" >"$work/stdout" 2>"$work/stderr"
}

copy_fails_with() {
  ! copy "$1" "$2" && grep -q "$3" "$work/stderr"
}

copy_is_identical() {
  copy "$1" "$2" && cmp -s "$1" "$2"
}

printf '{"scopeName": "source.nurt", "patterns": []}\n' >"$work/valid.json"
printf '{"scopeName": "source.other"}\n' >"$work/other.json"

expect 'copies the grammar unchanged and creates the target directory' \
  copy_is_identical "$work/valid.json" "$work/out/syntaxes/nurt.tmLanguage.json"
expect 'fails with a readable error when the grammar is missing' \
  copy_fails_with "$work/missing.json" "$work/out2/nurt.tmLanguage.json" 'grammar not found'
expect 'does not create the target when the grammar is missing' \
  test ! -e "$work/out2/nurt.tmLanguage.json"
expect 'rejects a grammar with another scope name' \
  copy_fails_with "$work/other.json" "$work/out3/nurt.tmLanguage.json" 'does not declare the scope source.nurt'

((failures == 0))

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
