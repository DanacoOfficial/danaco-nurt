#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Runs the cross-system tests of the system, log, network, schedule, service
# and program crates on Linux.
#
# Environment:
#   NURT_IT_SRC         root of a clone of the repository; default: the clone
#                       that contains this script
#   NURT_IT_OUT         directory of the results; default: target/it below
#                       NURT_IT_SRC
#   NURT_IT_SUITES      suites of matrix.toml separated by spaces or commas:
#                       container, system, scenarios; default: container
#   NURT_IT_PRIVILEGED  1 runs the tests that change the configuration of the
#                       system; default: 0
#   NURT_IT_NURT        program installed as the service by the service
#                       scenarios; without it these scenarios are skipped
#
# Results: test-<suite>.log, junit-<suite>.xml when cargo-nextest and the
# nextest profile `ci` are available, and summary.json. The exit code is 0
# when every suite passed.

set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
src=${NURT_IT_SRC:-$(cd "$script_dir/../../.." && pwd)}
out=${NURT_IT_OUT:-$src/target/it}
suites=${NURT_IT_SUITES:-container}
export NURT_IT_PRIVILEGED=${NURT_IT_PRIVILEGED:-0}

crates=(-p nurt-sys -p nurt-syslog -p nurt-net -p nurt-sched -p nurt-service -p nurt-programs)

user_unit="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/nurt-service.service"
system_unit=/etc/systemd/system/nurt-service.service
had_user_unit=0
had_system_unit=0
[[ -e $user_unit ]] && had_user_unit=1
[[ -e $system_unit ]] && had_system_unit=1

# Removes the service units that an interrupted run of the service scenarios
# left behind; units that existed before the run stay.
# shellcheck disable=SC2329 # runs through the trap below
cleanup() {
  if [[ $had_user_unit == 0 && -e $user_unit ]]; then
    systemctl --user disable --now nurt-service.service >/dev/null 2>&1 || true
    rm -f "$user_unit"
    systemctl --user daemon-reload >/dev/null 2>&1 || true
  fi
  if [[ $had_system_unit == 0 && -e $system_unit && $NURT_IT_PRIVILEGED == 1 ]]; then
    systemctl disable --now nurt-service.service >/dev/null 2>&1 || true
    rm -f "$system_unit"
    systemctl daemon-reload >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

has_nextest_profile() {
  cargo nextest --version >/dev/null 2>&1 &&
    grep -qs '^\[profile\.ci' "$src/.config/nextest.toml"
}

# Sums the counters of the `test result:` lines of a log.
counts() {
  awk '/^test result:/ {
         for (i = 1; i <= NF; i++) {
           if ($(i + 1) ~ /^passed/) p += $i
           if ($(i + 1) ~ /^failed/) f += $i
           if ($(i + 1) ~ /^ignored/) g += $i
         }
       }
       END { printf "\"passed\": %d, \"failed\": %d, \"ignored\": %d", p, f, g }' "$1"
}

run_suite() {
  local suite=$1
  local log="$out/test-$suite.log" status=0 args run_ignored
  : >"$log"
  case $suite in
    container) args=() run_ignored=default ;;
    system) args=(--include-ignored) run_ignored=all ;;
    scenarios) args=(--ignored) run_ignored=only ;;
    *)
      echo "unknown suite: $suite; expected container, system or scenarios" >&2
      return 2
      ;;
  esac
  if has_nextest_profile; then
    (cd "$src" && cargo nextest run --locked "${crates[@]}" --run-ignored "$run_ignored" \
      --profile ci) >"$log" 2>&1 || status=$?
    cp "$src/target/nextest/ci/junit.xml" "$out/junit-$suite.xml" 2>/dev/null || true
  fi
  (cd "$src" && cargo test --locked "${crates[@]}" -- ${args[@]+"${args[@]}"}) >>"$log" 2>&1 || status=$?
  printf '%s{"suite": "%s", "exit_code": %d, %s}' "$separator" "$suite" "$status" \
    "$(counts "$log")" >>"$out/summary.json"
  separator=", "
  return "$status"
}

mkdir -p "$out"
printf '{"system": "linux", "suites": [' >"$out/summary.json"
separator=""
result=0
for suite in ${suites//,/ }; do
  run_suite "$suite" || result=1
done
printf ']}\n' >>"$out/summary.json"
exit "$result"
