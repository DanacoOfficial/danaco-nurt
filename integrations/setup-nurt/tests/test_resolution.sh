#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           integrations/setup-nurt/tests/test_resolution.sh
# Responsibility:   Tests for SemVer comparison, range matching and version resolution.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

# Tests for SemVer comparison, range matching and version resolution.

set -uo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=harness.sh
source "$here/harness.sh"
# shellcheck source=../scripts/lib.sh
source "$here/../scripts/lib.sh"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

check_cmp() {
  assert_eq "semver_cmp $1 $2" "$3" "$(semver_cmp "$1" "$2")"
}

check_cmp 1.2.3 1.2.3 0
check_cmp 1.2.3 1.2.4 -1
check_cmp 1.10.0 1.9.0 1
check_cmp 2.0.0 1.99.99 1
check_cmp 1.0.0-rc.1 1.0.0 -1
check_cmp 1.0.0 1.0.0-rc.1 1
check_cmp 1.0.0-alpha 1.0.0-alpha.1 -1
check_cmp 1.0.0-alpha.2 1.0.0-alpha.10 -1
check_cmp 1.0.0-1 1.0.0-alpha -1
check_cmp 1.0.0+build.5 1.0.0 0
check_cmp v1.2.3 1.2.3 0
# SemVer orders identifiers by ASCII code: upper case before lower case, in
# every locale.
check_cmp 1.0.0-Alpha 1.0.0-alpha -1
check_cmp 1.0.0-beta 1.0.0-Beta 1
if locale -a 2>/dev/null | grep -qix 'en_US.utf-\?8'; then
  assert_eq 'semver_cmp ignores a locale with another collation' -1 \
    "$(LC_ALL=en_US.UTF-8 semver_cmp 1.0.0-Alpha 1.0.0-alpha)"
fi

assert_succeeds 'accepts plain SemVer' is_semver 1.2.3
assert_succeeds 'accepts SemVer with v prefix' is_semver v1.2.3
assert_succeeds 'accepts pre-release SemVer' is_semver 1.2.3-rc.1
assert_fails 'rejects two-part version' is_semver 1.2
assert_fails 'rejects latest keyword' is_semver latest
assert_fails 'rejects range text' is_semver '^1.2.3'

in_range() {
  assert_succeeds "$1 satisfies '$2'" range_satisfies "$1" "$2"
}

out_of_range() {
  assert_fails "$1 does not satisfy '$2'" range_satisfies "$1" "$2"
}

in_range 1.4.2 1
in_range 1.4.2 1.x
in_range 1.4.2 1.4
in_range 1.4.2 1.4.x
in_range 1.4.2 '*'
in_range 1.4.2 1.4.2
out_of_range 1.4.2 1.4.3
out_of_range 2.0.0 1
out_of_range 1.5.0 1.4
in_range 1.4.2 '^1.2.3'
out_of_range 2.0.0 '^1.2.3'
out_of_range 1.2.2 '^1.2.3'
in_range 0.2.9 '^0.2.3'
out_of_range 0.3.0 '^0.2.3'
in_range 0.0.3 '^0.0.3'
out_of_range 0.0.4 '^0.0.3'
in_range 1.2.9 '~1.2.3'
out_of_range 1.3.0 '~1.2.3'
in_range 1.9.0 '~1'
in_range 1.5.0 '>=1.2.0 <2.0.0'
out_of_range 2.0.0 '>=1.2.0 <2.0.0'
in_range 1.5.0 '>= 1.2.0 < 2.0.0'
in_range 2.5.0 '1.x || 2.x'
out_of_range 3.0.0 '1.x || 2.x'
in_range 1.3.0 '>1.2'
out_of_range 1.2.9 '>1.2'
in_range 1.2.9 '<=1.2'
out_of_range 1.3.0 '<=1.2'
in_range 1.2.3 '<=1.2.3'
out_of_range 1.2.3 '>1.2.3'
out_of_range 1.5.0-rc.1 '^1.2.0'
in_range 1.5.0-rc.1 '1.5.0-rc.1'
assert_fails 'rejects malformed range' range_satisfies 1.2.3 'abc'

cat >"$work/releases.json" <<'JSON'
[
  {"tag_name": "v1.3.0-rc.1", "prerelease": true},
  {"tag_name": "v1.2.10"},
  {"tag_name": "v1.2.9"},
  {"tag_name": "v1.10.0"},
  {"tag_name": "v1.9.4"},
  {"tag_name": "v2.0.0"},
  {"tag_name": "v2.1.0-beta.2", "prerelease": true},
  {"tag_name": "nightly"}
]
JSON

extract_versions "$work/releases.json" >"$work/versions.txt"
assert_eq 'extracts SemVer tags only' 7 "$(wc -l <"$work/versions.txt" | tr -d ' ')"
assert_eq 'select latest ignores pre-releases' 2.0.0 "$(select_version "$work/versions.txt" latest)"
assert_eq 'select caret range' 1.10.0 "$(select_version "$work/versions.txt" '^1.2.0')"
assert_eq 'select tilde range' 1.2.10 "$(select_version "$work/versions.txt" '~1.2.0')"
assert_eq 'select partial version' 1.10.0 "$(select_version "$work/versions.txt" 1)"
assert_eq 'select bounded range' 1.9.4 "$(select_version "$work/versions.txt" '>=1.3.0 <1.10.0')"
assert_fails 'select fails without a match' select_version "$work/versions.txt" '>=3.0.0'

NURT_RELEASES_URL="file://$work/releases.json"
export NURT_RELEASES_URL
assert_eq 'resolve latest from listing' 2.0.0 "$(resolve_version latest)"
assert_eq 'resolve range from listing' 1.10.0 "$(resolve_version '1.x')"
assert_eq 'resolve exact version without listing' 9.9.9 "$(NURT_RELEASES_URL=file:///nonexistent resolve_version 9.9.9)"
assert_eq 'resolve strips v prefix' 1.2.3 "$(resolve_version v1.2.3)"
assert_eq 'resolve exact pre-release' 3.0.0-rc.1 "$(resolve_version 3.0.0-rc.1)"
assert_fails 'resolve fails without a match' resolve_version '>=5.0.0'
unavailable_listing() {
  env NURT_RELEASES_URL=file:///nonexistent bash -c 'source "$1"; resolve_version latest' _ "$here/../scripts/lib.sh"
}
assert_fails 'resolve fails when listing is unavailable' unavailable_listing
out=$(unavailable_listing 2>&1 || true)
assert_contains 'explains the unavailable listing' 'cannot download the release listing' "$out"
out=$( (resolve_version '>=5.0.0') 2>&1 || true)
assert_contains 'explains the unmatched range' "no release satisfies version '>=5.0.0'" "$out"

finish_tests

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
