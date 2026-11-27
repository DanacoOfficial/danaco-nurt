#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Functional scenarios of the release check.
#
# Repeats the scenarios T01 to T19 of the 0.1.0 verification, the elevated
# scenarios T20 (Linux, needs sudo without a password) and T21 (Windows, needs an
# elevated console) and the scenario 2.11 (`options.base` of `nurt serve`) against a built executable. Every
# scenario runs in its own empty directory with its own HOME, XDG_STATE_HOME
# and XDG_CONFIG_HOME. The directories lie under $TMPDIR, so they share the
# journal of that volume, as scenario T01 requires.
#
# Usage:
#
#   tools/release/scenarios.sh <path-to-nurt> <results-dir>
#
# The results directory receives results.tsv (identifier, status, reason,
# separated by tabs), table.txt and one <id>.log per scenario. The status is
# PASS, FAIL or SKIP; SKIP is used only for scenarios that need Windows or
# macOS. The table is printed at the end.
#
# Environment (all optional):
#
#   NURT_SCENARIO_ONLY      space-separated identifiers to run, for example "T01 T03"
#   NURT_SCENARIO_TIMEOUT   limit of one command in seconds (120)
#   NURT_SCENARIO_FILES     number of files of scenario T12 (3000)
#   NURT_SCENARIO_LIMIT     limit in seconds of `replace` and of `undo` in T12 (60)
#
# Exit code: 0 without FAIL, 1 with at least one FAIL, 2 for a usage error.

set -u

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ $# -ne 2 ] || [ "$1" = "-h" ] || [ "$1" = "--help" ]; then
  sed -n '/^# Usage:/,/^# Exit code/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
  exit 2
fi
[ -x "$1" ] || { echo "scenarios: not an executable: $1" >&2; exit 2; }
NURT="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
mkdir -p "$2" || exit 2
RESULTS="$(cd "$2" && pwd)"
: > "$RESULTS/results.tsv"

TIMEOUT="${NURT_SCENARIO_TIMEOUT:-120}"
FILES="${NURT_SCENARIO_FILES:-3000}"
LIMIT="${NURT_SCENARIO_LIMIT:-60}"
mkdir -p "${TMPDIR:-/tmp}" || exit 2
BASE="$(mktemp -d "${TMPDIR:-/tmp}/nurt-scenarios-XXXXXX")" || exit 2
trap 'rm -rf "$BASE"' EXIT

# --- helpers used inside a scenario (each scenario runs in a subshell) ------

# Result of the scenario: reason in one line, then leave the subshell.
ok() { printf '%s\n' "$*" > "$REASON"; exit 0; }
fail() { printf '%s\n' "$*" > "$REASON"; exit 1; }
skip() { printf '%s\n' "$*" > "$REASON"; exit 77; }

# Run nurt with the given arguments; sets OUT (stdout and stderr) and RC.
n() {
  {
    printf '$ nurt'
    printf ' %q' "$@"
    printf '\n'
  } >> "$LOG"
  OUT="$(timeout "$TIMEOUT" "$NURT" "$@" 2>&1 < /dev/null)"
  RC=$?
  printf '%s\nrc=%s\n' "$OUT" "$RC" >> "$LOG"
}

# Run nurt with a different HOME: h <dir> args...
h() {
  local dir="$1"
  shift
  mkdir -p "$dir"
  HOME="$dir" XDG_STATE_HOME="$dir/state" XDG_CONFIG_HOME="$dir/config" n "$@"
}

# Run a shell command and record it in the log; sets OUT and RC.
sh_run() {
  printf '$ %s\n' "$*" >> "$LOG"
  OUT="$(timeout "$TIMEOUT" "$@" 2>&1 < /dev/null)"
  RC=$?
  printf '%s\nrc=%s\n' "$OUT" "$RC" >> "$LOG"
}

# Number of the operation of a result ("op 12  write  ok").
op_number() { printf '%s\n' "$OUT" | sed -n 's/^op \([0-9][0-9]*\) .*/\1/p' | head -1; }

now() { date +%s.%N; }
seconds_since() { awk -v a="$1" -v b="$(now)" 'BEGIN { printf "%.1f", b - a }'; }
over() { awk -v a="$1" -v b="$2" 'BEGIN { exit !(a > b) }'; }

mtime() { stat -c %Y "$1"; }
mode_of() { stat -c %a "$1"; }

# --- scenarios ---------------------------------------------------------------

scn_T01() {
  local a="$PWD/a" b="$PWD/b" numbers="" number i
  mkdir -p "$a" "$b"
  printf 'one\n' > "$a/fa.txt"
  printf 'one\n' > "$b/fb.txt"
  for i in 1 2 3; do
    h "$PWD/home-a" "write \"$a/fa.txt\" text: \"a$i\" mode: overwrite"
    [ "$RC" -eq 0 ] || fail "write $i from the first HOME ended with $RC"
    case "$OUT" in *journal_error*) fail "write $i from the first HOME: journal_error" ;; esac
    numbers="$numbers $(op_number)"
  done
  h "$PWD/home-b" "write \"$b/fb.txt\" text: \"b1\" mode: overwrite"
  [ "$RC" -eq 0 ] || fail "write from a new HOME ended with $RC"
  case "$OUT" in *journal_error*) fail "write from a new HOME: journal_error" ;; esac
  number="$(op_number)"
  case " $numbers " in
    *" $number "*) fail "operation number $number of the new HOME was already used by the first HOME" ;;
  esac
  h "$PWD/home-b" undo
  [ "$RC" -eq 0 ] || fail "undo from the new HOME ended with $RC"
  [ "$(cat "$b/fb.txt")" = "one" ] || fail "undo from the new HOME did not restore its file"
  [ "$(cat "$a/fa.txt")" = "a3" ] || fail "undo from the new HOME changed the file of the first HOME"
  ok "numbers$numbers $number are distinct; undo reverted only the own operation"
}

scn_T02() {
  printf 'foo\n' > "$PWD/o.txt"
  touch -d '2001-01-01 00:00:00' "$PWD/o.txt"
  n "write \"$PWD/o.txt\" text: \"new\" mode: overwrite"
  [ "$RC" -eq 0 ] || fail "write overwrite ended with $RC"
  over "$(mtime "$PWD/o.txt")" 1700000000 || fail "write overwrite kept the old modification time"
  printf 'foo\n' > "$PWD/r.txt"
  touch -d '2001-01-01 00:00:00' "$PWD/r.txt"
  n "replace \"foo\" \"bar\" in: [\"$PWD/r.txt\"]"
  [ "$RC" -eq 0 ] || fail "replace ended with $RC"
  over "$(mtime "$PWD/r.txt")" 1700000000 || fail "replace kept the old modification time"
  ok "write overwrite and replace set the modification time to the time of the write"
}

scn_T03() {
  printf 'x\n' > "$PWD/f.txt"
  mkdir -p "$PWD/d1/sub" "$PWD/dest"
  printf 'y\n' > "$PWD/d1/sub/y.txt"
  n "move \"$PWD/f.txt\" to: \"$PWD/g.txt\""
  [ "$RC" -eq 0 ] && [ -f "$PWD/g.txt" ] || fail "move of a file ended with $RC: $(printf '%s' "$OUT" | head -1)"
  n undo
  [ "$RC" -eq 0 ] && [ -f "$PWD/f.txt" ] || fail "undo of the move of a file ended with $RC: $(printf '%s' "$OUT" | head -1)"
  n "move \"$PWD/d1\" to: \"$PWD/d2\""
  [ "$RC" -eq 0 ] && [ -f "$PWD/d2/sub/y.txt" ] || fail "move of a directory ended with $RC: $(printf '%s' "$OUT" | head -1)"
  n undo
  [ "$RC" -eq 0 ] && [ -f "$PWD/d1/sub/y.txt" ] || fail "undo of the move of a directory ended with $RC"
  n "move \"$PWD/f.txt\" into: \"$PWD/dest\""
  [ "$RC" -eq 0 ] && [ -f "$PWD/dest/f.txt" ] || fail "move into a directory ended with $RC: $(printf '%s' "$OUT" | head -1)"
  n undo
  [ "$RC" -eq 0 ] && [ -f "$PWD/f.txt" ] || fail "undo of the move into a directory ended with $RC"
  ok "move of a file, of a directory and into a directory, and their undo, end with 0"
}

scn_T04() {
  sh_run timeout 30 "$NURT" mcp --stdio
  [ "$RC" -ne 124 ] || fail "mcp --stdio did not end at the end of the input"
  [ "$RC" -eq 0 ] || fail "mcp --stdio ended with $RC, expected 0"
  sh_run timeout 30 "$NURT" lsp
  [ "$RC" -ne 124 ] || fail "lsp did not end at the end of the input"
  { [ "$RC" -eq 1 ] || [ "$RC" -eq 0 ]; } || fail "lsp ended with $RC, expected 1 (end of input without shutdown)"
  ok "mcp --stdio ends with 0 and lsp with $RC at the end of the input"
}

scn_T05() { skip "needs Windows: PowerShell aliases Move-Item and Write-Output"; }

scn_T06() {
  printf 'secret\n' > "$PWD/s.txt"
  chmod 600 "$PWD/s.txt"
  touch -d '2001-02-03 04:05:06' "$PWD/s.txt"
  local before xattr=no
  before="$(mtime "$PWD/s.txt")"
  if python3 -I -c 'import os, sys; os.setxattr(sys.argv[1], "user.nurt-scenario", b"v")' "$PWD/s.txt" 2> /dev/null; then
    xattr=yes
  fi
  n "remove \"$PWD/s.txt\""
  [ "$RC" -eq 0 ] || fail "remove ended with $RC"
  n undo
  [ "$RC" -eq 0 ] || fail "undo of remove ended with $RC"
  [ -f "$PWD/s.txt" ] || fail "undo did not restore the file"
  [ "$(mode_of "$PWD/s.txt")" = "600" ] || fail "restored mode is $(mode_of "$PWD/s.txt"), expected 600"
  [ "$(mtime "$PWD/s.txt")" = "$before" ] || fail "restored modification time differs"
  if [ "$xattr" = yes ]; then
    python3 -I -c 'import os, sys; sys.exit(0 if os.getxattr(sys.argv[1], "user.nurt-scenario") == b"v" else 1)' "$PWD/s.txt" 2> /dev/null \
      || fail "restored file lost its extended attribute"
  fi
  ok "undo of remove restored mode 600 and modification time (extended attributes checked: $xattr)"
}

scn_T07() {
  printf 'keep\n' > "$PWD/k.txt"
  n "remve \"$PWD/k.txt\""
  [ -f "$PWD/k.txt" ] || fail "a misspelled destructive command was executed without confirmation"
  [ "$RC" -ne 0 ] || fail "a misspelled destructive command ended with 0 although nothing ran"
  ok "remve was not executed (exit $RC: $(printf '%s' "$OUT" | head -1 | cut -c1-80))"
}

scn_T08() {
  printf '#!/bin/bash\n# first line\n# second line\n# third line\necho "hello"\ncp "%s/a.txt" "%s/b.txt"\n' "$PWD" "$PWD" > "$PWD/in.sh"
  n migrate "$PWD/in.sh"
  [ "$RC" -eq 0 ] || fail "migrate ended with $RC: $(printf '%s' "$OUT" | grep -v '^#' | head -1 | cut -c1-100)"
  case "$OUT" in *[Cc]opyright*) fail "the result carries a copyright header" ;; esac
  # The translated script is the text before the report that follows it.
  printf '%s\n' "$OUT" | sed '/^script /,$d' > "$PWD/out.nurt"
  n check "$PWD/out.nurt"
  case "$OUT" in *E_SYNTAX*) fail "the migrated script has a syntax error: $(printf '%s' "$OUT" | head -1 | cut -c1-100)" ;; esac
  [ "$RC" -eq 0 ] || fail "check of the migrated script ended with $RC: $(printf '%s' "$OUT" | head -1 | cut -c1-100)"
  ok "the migrated script has no syntax error and no copyright header"
}

scn_T09() {
  mkdir -p "$PWD/s"
  printf 'a 123 b\n' > "$PWD/s/x.txt"
  n "search \"\\d+\" \"$PWD/s\""
  [ "$RC" -eq 0 ] || fail "search with \\d in a text ended with $RC: $(printf '%s' "$OUT" | head -1 | cut -c1-100)"
  case "$OUT" in *"matched 1"*) ;; *) fail "search with \\d found no match" ;; esac
  ok "regular expression escapes are accepted (the Windows path part of T09 needs Windows)"
}

scn_T10() { skip "needs Windows: the registry: scheme"; }

scn_T11() {
  printf 'alpha-marker\n' > "$PWD/p1.txt"
  printf 'beta-marker\n' > "$PWD/p2.txt"
  printf 'read "%s/p1.txt"\nread "%s/p2.txt"\n' "$PWD" "$PWD" > "$PWD/two.nurt"
  n script "$PWD/two.nurt"
  [ "$RC" -eq 0 ] || fail "script ended with $RC"
  case "$OUT" in *alpha-marker*) ;; *) fail "the result of the first pipeline is missing" ;; esac
  case "$OUT" in *beta-marker*) ;; *) fail "the result of the second pipeline is missing" ;; esac
  ok "the script printed the results of both pipelines"
}

scn_T12() {
  local d="$PWD/many" i start took undo_took
  mkdir -p "$d"
  for i in $(seq 1 "$FILES"); do printf 'foo 123456789\n' > "$d/f$i.txt"; done
  start="$(now)"
  n "search \"foo\" \"$d\" |> replace \"bar\""
  took="$(seconds_since "$start")"
  [ "$RC" -eq 0 ] || fail "replace over $FILES files ended with $RC: $(printf '%s' "$OUT" | head -1 | cut -c1-100)"
  [ "$(grep -l bar "$d"/f1.txt "$d/f$FILES.txt" | wc -l)" -eq 2 ] || fail "replace did not change the files"
  over "$took" "$LIMIT" && fail "replace over $FILES files took ${took}s, limit ${LIMIT}s"
  start="$(now)"
  n undo
  undo_took="$(seconds_since "$start")"
  [ "$RC" -eq 0 ] || fail "undo over $FILES files ended with $RC"
  over "$undo_took" "$LIMIT" && fail "undo over $FILES files took ${undo_took}s, limit ${LIMIT}s"
  [ "$(cat "$d/f1.txt")" = "foo 123456789" ] || fail "undo did not restore the files"
  ok "replace ${took}s and undo ${undo_took}s over $FILES files (limit ${LIMIT}s)"
}

scn_T13() {
  n "run [\"sh\",\"-c\",\"exit 3\"] cwd: \"$PWD\""
  [ "$RC" -eq 0 ] || fail "run of a program with exit code 3 ended with $RC, the contract is 0 with a note"
  case "$OUT" in *exit_nonzero*) ;; *) fail "the note exit_nonzero is missing" ;; esac
  n "run [\"sh\",\"-c\",\"exit 3\"] cwd: \"$PWD\" |> assert exit_code: 0"
  [ "$RC" -ne 0 ] || fail "assert exit_code: 0 did not fail for exit code 3"
  ok "exit code 3 gives 0 with the note exit_nonzero; assert makes it an error"
}

scn_T14() {
  mkdir -p "$PWD/gone"
  n "rm -rf $PWD/gone"
  [ "$RC" -eq 0 ] && [ ! -e "$PWD/gone" ] || fail "rm -rf was not translated and executed: $(printf '%s' "$OUT" | head -1 | cut -c1-100)"
  n "curl -s https://nurt-scenario.invalid/"
  case "$OUT" in *E_UNKNOWN_PARAM*) fail "curl -s was not translated: E_UNKNOWN_PARAM" ;; esac
  case "$OUT" in *http.request*) ;; *) fail "curl -s was not translated to http.request" ;; esac
  ok "rm -rf and curl -s are translated"
}

scn_T15() {
  n check -e "find \"$PWD\" |> filter size > \"abc\""
  [ "$RC" -ne 0 ] || fail "check accepts filter size > \"abc\""
  n check --format json -e "find \"$PWD\" |> filter size > \"abc\""
  [ "$RC" -ne 0 ] || fail "check --format json accepts filter size > \"abc\""
  printf '%s' "$OUT" | python3 -I -c 'import json, sys; d = json.load(sys.stdin); sys.exit(0 if "error" in d else 1)' 2> /dev/null \
    || fail "check --format json does not print a JSON document with an error"
  ok "check rejects a wrong field type and prints the error as JSON"
}

scn_T16() {
  mkdir -p "$PWD/raw"
  printf 'x\n' > "$PWD/raw/$(printf 'bad\377name.txt')"
  n --json "find \"$PWD/raw\""
  [ "$RC" -eq 0 ] || fail "find in a directory with a non-UTF-8 name ended with $RC"
  case "$OUT" in *raw_name*) ;; *) fail "the record of the entry has no raw_name" ;; esac
  n --json "find \"$PWD/raw\" |> remove"
  [ "$RC" -eq 0 ] || fail "remove of the file ended with $RC: $(printf '%s' "$OUT" | head -1 | cut -c1-100)"
  case "$OUT" in *'"raw'*) ;; *) fail "the paths of the changes in the report have no raw form" ;; esac
  ok "records and change paths carry the raw form of a non-UTF-8 name"
}

scn_T17() {
  local plain_lines nl_lines
  mkdir -p "$PWD/t/a/b/c" "$PWD/p2"
  printf 'x\n' > "$PWD/t/a/b/c/x.txt"
  n "find \"$PWD/t\" glob: \"**/*.txt\""
  [ "$RC" -eq 0 ] || fail "find ended with $RC"
  printf '%s\n' "$OUT" | grep -q 'a/' && printf '%s\n' "$OUT" | grep -q 'b/' && printf '%s\n' "$OUT" | grep -q 'c/' \
    || fail "the tree view omits intermediate directories"
  printf 'x\n' > "$PWD/p2/$(printf 'a\nb')"
  n "find \"$PWD/p2\""
  [ "$RC" -eq 0 ] || fail "find of a name with a line feed ended with $RC"
  if printf '%s\n' "$OUT" | grep -q '^b'; then fail "a name with a line feed breaks the view: a line starts with the second part of the name"; fi
  printf 'l1\nl2\nl3\n' > "$PWD/m.txt"
  printf 'l1\n' > "$PWD/o.txt"
  n "read \"$PWD/m.txt\""
  plain_lines="$(printf '%s\n' "$OUT" | wc -l)"
  n "read \"$PWD/o.txt\""
  nl_lines="$(printf '%s\n' "$OUT" | wc -l)"
  [ "$plain_lines" -eq "$nl_lines" ] || fail "multi-line content of read spreads over several lines of the table"
  ok "tree with intermediate directories; names with a line feed and multi-line content keep the view"
}

scn_T18() {
  n doctor
  [ "$RC" -eq 0 ] || fail "doctor ended with $RC"
  printf '%s\n' "$OUT" | grep -Eq 'features +ok +.*mcp.*lsp.*migrate.*service.*journal' \
    || fail "the features row does not list mcp, lsp, migrate, service and journal"
  ok "doctor lists the linked components"
}

scn_T19() {
  local preview executed
  mkdir -p "$PWD/tree/inner"
  printf 'one\n' > "$PWD/tree/one.txt"
  printf 'two\n' > "$PWD/tree/inner/two.txt"
  n --preview "remove \"$PWD/tree\""
  [ "$RC" -eq 0 ] || fail "preview ended with $RC"
  [ -d "$PWD/tree/inner" ] || fail "preview changed the tree"
  preview="$(printf '%s\n' "$OUT" | grep '^targets' | head -1)"
  n "remove \"$PWD/tree\""
  [ "$RC" -eq 0 ] || fail "remove ended with $RC"
  executed="$(printf '%s\n' "$OUT" | grep '^targets' | head -1)"
  [ "$preview" = "$executed" ] || fail "preview says \"$preview\", execution says \"$executed\""
  n undo
  [ "$RC" -eq 0 ] || fail "undo ended with $RC"
  [ -f "$PWD/tree/one.txt" ] && [ -f "$PWD/tree/inner/two.txt" ] || fail "undo did not restore the files and the directories"
  ok "preview and execution agree (\"$preview\"); undo restores the tree"
}

scn_T20() {
  local target="/etc/nurt-scenario-T20.conf" number
  [ "$(uname -s)" = "Linux" ] || skip "needs Linux"
  command -v sudo > /dev/null 2>&1 || skip "needs sudo"
  sudo -n true > /dev/null 2>&1 || skip "needs sudo without a password"
  sudo -n rm -f "$target"
  n "write \"$target\" text: \"a=1\\n\" mode: overwrite privilege: elevated"
  [ "$RC" -eq 0 ] || fail "elevated write ended with $RC"
  [ -f "$target" ] || fail "elevated write did not create the file"
  [ "$(stat -c %U "$target")" = "root" ] || fail "the file is not owned by root"
  case "$OUT" in *elevated*sudo* | *sudo*elevated*) ;; *) fail "the report does not state the elevation and the backend" ;; esac
  number="$(op_number)"
  [ -n "$number" ] || fail "the report has no operation number"
  n "undo $number"
  [ "$RC" -eq 0 ] || fail "undo of the elevated entry ended with $RC"
  [ ! -e "$target" ] || { sudo -n rm -f "$target"; fail "undo did not remove the file"; }
  ok "elevated write through sudo, entry $number undone through the backend"
}

scn_T21() {
  local target number
  case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) ;; *) skip "needs Windows" ;; esac
  target="C:/Program Files/nurt-scenario-T21.txt"
  n "write \"$target\" text: \"a\" mode: overwrite privilege: elevated"
  [ "$RC" -eq 0 ] || fail "elevated write into Program Files ended with $RC"
  number="$(op_number)"
  n "undo $number"
  [ "$RC" -eq 0 ] || fail "undo of the elevated entry ended with $RC"
  [ ! -e "$target" ] || fail "undo did not remove the file"
  ok "elevated write into Program Files with the self backend, entry $number undone"
}

scn_2_11() {
  mkdir -p "$PWD/cfg"
  printf 'a = 1\n' > "$PWD/cfg/c.toml"
  printf '$ nurt serve --stdio (options.base = %s)\n' "$PWD/cfg" >> "$LOG"
  OUT="$(python3 -I - "$NURT" "$PWD/cfg" "$TIMEOUT" << 'PY' 2>&1
import json
import subprocess
import sys

nurt, base, limit = sys.argv[1], sys.argv[2], int(sys.argv[3])
messages = [
    {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocol": "nurt-rpc/1"}},
    {"jsonrpc": "2.0", "id": 2, "method": "run",
     "params": {"nurt": "1.0", "program": 'read "c.toml"', "options": {"base": base}}},
]
stdin = "".join(json.dumps(m) + "\n" for m in messages)
done = subprocess.run([nurt, "serve", "--stdio"], input=stdin, capture_output=True, text=True, timeout=limit)
print(done.stdout)
reply = {}
for line in done.stdout.splitlines():
    document = json.loads(line)
    reply[document.get("id")] = document
report = reply.get(2, {}).get("result", {}).get("report", {})
if report.get("status") != "ok":
    print("RESULT status", report.get("status"), json.dumps(report.get("error", reply.get(2)))[:200])
    sys.exit(1)
print("RESULT ok")
PY
)"
  RC=$?
  printf '%s\nrc=%s\n' "$OUT" "$RC" >> "$LOG"
  [ "$RC" -eq 0 ] || fail "$(printf '%s\n' "$OUT" | grep '^RESULT' | head -1 | cut -c1-140)"
  ok "read \"c.toml\" with options.base binds against the base"
}

# --- runner ------------------------------------------------------------------

scenarios="T01 T02 T03 T04 T05 T06 T07 T08 T09 T10 T11 T12 T13 T14 T15 T16 T17 T18 T19 T20 T21 2.11"
if [ -n "${NURT_SCENARIO_ONLY:-}" ]; then scenarios="$NURT_SCENARIO_ONLY"; fi

for id in $scenarios; do
  function_name="scn_${id//./_}"
  if ! declare -F "$function_name" > /dev/null; then
    echo "scenarios: unknown scenario $id" >&2
    exit 2
  fi
  dir="$BASE/$id"
  mkdir -p "$dir/work" "$dir/account"
  (
    cd "$dir/work" || exit 1
    export HOME="$dir/account" XDG_STATE_HOME="$dir/account/state" XDG_CONFIG_HOME="$dir/account/config"
    export XDG_DATA_HOME="$dir/account/data" XDG_CACHE_HOME="$dir/account/cache" LANG=C.UTF-8
    export LOG="$RESULTS/$id.log" REASON="$dir/reason"
    : > "$LOG"
    "$function_name"
    fail "scenario $id ended without a result"
  )
  code=$?
  case "$code" in
    0) status=PASS ;;
    77) status=SKIP ;;
    *) status=FAIL ;;
  esac
  reason="$(tr '\n\t' '  ' < "$dir/reason" 2> /dev/null)"
  [ -n "$reason" ] || reason="scenario ended with code $code without a reason"
  printf '%s\t%s\t%s\n' "$id" "$status" "$reason" >> "$RESULTS/results.tsv"
  rm -rf "$dir"
done

python3 -I "$here/scenario_table.py" "$RESULTS/results.tsv" | tee "$RESULTS/table.txt"
exit "${PIPESTATUS[0]}"
