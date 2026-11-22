#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Copies the TextMate grammar of Danaco Nurt into the extension.
#
# The grammar is maintained with the language server in editors/textmate of
# the repository and is copied unchanged. NURT_GRAMMAR_SOURCE overrides the
# source path and NURT_GRAMMAR_TARGET overrides the destination.

set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
extension_root=$(cd "$here/.." && pwd)
repository_root=$(cd "$extension_root/../.." && pwd)

source_file=${NURT_GRAMMAR_SOURCE:-$repository_root/editors/textmate/nurt.tmLanguage.json}
target_file=${NURT_GRAMMAR_TARGET:-$extension_root/syntaxes/nurt.tmLanguage.json}

if [[ ! -f $source_file ]]; then
  printf 'error: grammar not found: %s\n' "$source_file" >&2
  printf 'the grammar is built with the language server; check out editors/textmate or set NURT_GRAMMAR_SOURCE\n' >&2
  exit 1
fi

if ! grep -q '"scopeName"[[:space:]]*:[[:space:]]*"source\.nurt"' "$source_file"; then
  printf 'error: %s does not declare the scope source.nurt\n' "$source_file" >&2
  exit 1
fi

mkdir -p "$(dirname "$target_file")"
cp "$source_file" "$target_file"
printf 'copied %s to %s\n' "$source_file" "$target_file"
