#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           integrations/setup-nurt/tests/harness.sh
# Responsibility:   Minimal assertion harness shared by the setup-nurt tests.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.2, 2026-11-28
# License:          see the LICENSE file or https://www.danaco-group.pl

# Minimal assertion harness shared by the setup-nurt tests.

TESTS_RUN=0
TESTS_FAILED=0

# Prints the file:// URL of path $1. On Windows the path is converted to the
# form with a drive letter, because curl is a native program there and cannot
# resolve a POSIX path such as /tmp/... inside a file:// URL.
file_url() {
  local path=$1
  if command -v cygpath >/dev/null 2>&1; then
    path=$(cygpath -m "$path")
  fi
  case $path in
    /*) printf 'file://%s\n' "$path" ;;
    *) printf 'file:///%s\n' "$path" ;;
  esac
}

# Prints path $1 in the form the action writes to its outputs and to
# GITHUB_PATH: with a drive letter and backslashes on Windows, unchanged
# elsewhere.
native_path() {
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -w "$1"
  else
    printf '%s\n' "$1"
  fi
}

pass_test() {
  TESTS_RUN=$((TESTS_RUN + 1))
  printf 'ok   %s\n' "$1"
}

fail_test() {
  TESTS_RUN=$((TESTS_RUN + 1))
  TESTS_FAILED=$((TESTS_FAILED + 1))
  printf 'FAIL %s\n' "$1"
  shift
  if (($# > 0)); then
    printf '     %s\n' "$@"
  fi
}

assert_eq() {
  local name=$1 expected=$2 actual=$3
  if [[ $expected == "$actual" ]]; then
    pass_test "$name"
  else
    fail_test "$name" "expected: $expected" "actual:   $actual"
  fi
}

assert_succeeds() {
  local name=$1
  shift
  if ( "$@" ) >/dev/null 2>&1; then
    pass_test "$name"
  else
    fail_test "$name" "command failed: $*"
  fi
}

assert_fails() {
  local name=$1
  shift
  if ( "$@" ) >/dev/null 2>&1; then
    fail_test "$name" "command succeeded: $*"
  else
    pass_test "$name"
  fi
}

assert_file() {
  if [[ -f $2 ]]; then
    pass_test "$1"
  else
    fail_test "$1" "missing file: $2"
  fi
}

assert_no_file() {
  if [[ ! -e $2 ]]; then
    pass_test "$1"
  else
    fail_test "$1" "unexpected path: $2"
  fi
}

assert_contains() {
  local name=$1 needle=$2 haystack=$3
  if [[ $haystack == *"$needle"* ]]; then
    pass_test "$name"
  else
    fail_test "$name" "missing text: $needle" "in: $haystack"
  fi
}

finish_tests() {
  printf '%d tests, %d failed\n' "$TESTS_RUN" "$TESTS_FAILED"
  ((TESTS_FAILED == 0))
}

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
