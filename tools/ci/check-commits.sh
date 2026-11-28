#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           tools/ci/check-commits.sh
# Responsibility:   Checks the commits of a revision range against the contribution rules.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.2, 2026-11-28
# License:          see the LICENSE file or https://www.danaco-group.pl

#
# Checks the commits of a revision range against the contribution rules:
#   - the subject follows Conventional Commits 1.0.0 with an allowed type and
#     scope, and is at most 72 characters long;
#   - the message contains a "Signed-off-by" trailer of the commit author
#     (Developer Certificate of Origin 1.1).
#
# Usage: tools/ci/check-commits.sh <revision-range>
# Example: tools/ci/check-commits.sh origin/main..HEAD
#
# Environment:
#   NURT_COMMITS_ALLOW_FILE   list of already published commits that are
#                             exempt from the checks because their history is
#                             not rewritten (default tools/ci/commits-allow.txt).
#                             Each line is a full 40-character commit
#                             identifier, optionally followed by a comment;
#                             lines starting with "#" and empty lines are
#                             ignored.
#
# Allowed scopes are the names of the crates in "crates/" without the "nurt-"
# prefix and the repository areas listed in AREAS below.
#
# Exit status: 0 all commits valid, 1 invalid commits, 2 usage error.

set -eu

readonly TYPES="feat|fix|perf|refactor|docs|test|build|ci|style|chore|revert"
readonly AREAS="workspace deps ci spec catalog tables docs tools release repo bindings packaging integrations"
readonly MAX_SUBJECT=72

if [ $# -ne 1 ]; then
  echo "Usage: check-commits.sh <revision-range>" >&2
  exit 2
fi
range=$1

root=$(git rev-parse --show-toplevel)
cd "$root"

allow_file=${NURT_COMMITS_ALLOW_FILE:-tools/ci/commits-allow.txt}
allowed=" "
if [ -f "$allow_file" ]; then
  allowed=" $(sed -nE 's/^([0-9a-f]{40})([[:space:]].*)?$/\1/p' "$allow_file" | tr '\n' ' ')"
fi

scopes=" $AREAS "
for dir in crates/*/; do
  [ -d "$dir" ] || continue
  name=$(basename "$dir")
  scopes="$scopes${name#nurt-} "
done

errors=0
fail() {
  errors=$((errors + 1))
  printf '%s: %s\n' "$1" "$2" >&2
}

commits=$(git rev-list --no-merges --reverse "$range")
for commit in $commits; do
  case "$allowed" in
    *" $commit "*) continue ;;
  esac
  short=$(git rev-parse --short "$commit")
  subject=$(git log -1 --format=%s "$commit")
  author_email=$(git log -1 --format=%ae "$commit")
  trailers=$(git log -1 --format='%(trailers:key=Signed-off-by,valueonly)' "$commit")

  if [ "${#subject}" -gt "$MAX_SUBJECT" ]; then
    fail "$short" "subject longer than $MAX_SUBJECT characters"
  fi

  if printf '%s\n' "$subject" | grep -qE "^($TYPES)(\([a-z0-9-]+\))?!?: ([^ .]|[^ ].*[^.])$"; then
    scope=$(printf '%s\n' "$subject" | sed -nE 's/^[a-z]+\(([a-z0-9-]+)\).*/\1/p')
    if [ -n "$scope" ]; then
      case "$scopes" in
        *" $scope "*) ;;
        *) fail "$short" "unknown scope '$scope'" ;;
      esac
    fi
  else
    fail "$short" "subject does not follow '<type>(<scope>): <summary>': $subject"
  fi

  if ! printf '%s\n' "$trailers" | grep -qF "<$author_email>"; then
    fail "$short" "missing 'Signed-off-by' trailer of the author <$author_email>"
  fi
done

if [ "$errors" -ne 0 ]; then
  echo "check-commits: $errors problem(s) found" >&2
  exit 1
fi
echo "check-commits: all commits in $range are valid"

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
