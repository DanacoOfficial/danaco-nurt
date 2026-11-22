#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Runs a built image and checks the executable, the license files, the user
# and the state directory. Usage: smoke.sh IMAGE [ci|devcontainer|nurt].

set -uo pipefail
image=${1:?usage: smoke.sh IMAGE [nurt|ci|devcontainer]}
kind=${2:-nurt}
failures=0

check() {
  local description=$1
  shift
  if "$@" >/dev/null 2>&1; then
    printf 'ok   %s\n' "$description"
  else
    printf 'FAIL %s\n' "$description"
    failures=$((failures + 1))
  fi
}

run_nurt() {
  docker run --rm --entrypoint /usr/local/bin/nurt "$image" "$@"
}

version_output=$(run_nurt version 2>&1)
case $version_output in
  'Danaco Nurt '*) printf 'ok   version output\n' ;;
  *)
    printf 'FAIL version output: %s\n' "$version_output"
    failures=$((failures + 1))
    ;;
esac

check 'state directory is set' \
  test "$(docker inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$image" | grep -c '^NURT_STATE_DIR=/var/lib/nurt$')" -eq 1
check 'license label' \
  test "$(docker inspect --format '{{index .Config.Labels "org.opencontainers.image.licenses"}}' "$image")" = Apache-2.0

case $kind in
  nurt)
    check 'runs as an unprivileged user' \
      test "$(docker inspect --format '{{.Config.User}}' "$image")" = 65532:65532
    ;;
  ci)
    check 'runs as an unprivileged user' \
      test "$(docker inspect --format '{{.Config.User}}' "$image")" = 10001:10001
    check 'git is installed' docker run --rm --entrypoint git "$image" --version
    check 'license files are present' \
      docker run --rm --entrypoint test "$image" -f /usr/share/doc/nurt/LICENSE -a -f /usr/share/doc/nurt/NOTICE
    check 'state directory is writable' \
      docker run --rm --entrypoint touch "$image" /var/lib/nurt/probe
    ;;
  devcontainer)
    check 'git is installed' docker run --rm --entrypoint git "$image" --version
    check 'license files are present' \
      docker run --rm --entrypoint test "$image" -f /usr/share/doc/nurt/LICENSE -a -f /usr/share/doc/nurt/NOTICE
    ;;
  *)
    printf 'unknown image kind: %s\n' "$kind" >&2
    exit 2
    ;;
esac

((failures == 0))
