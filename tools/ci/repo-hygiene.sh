#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Repository hygiene check.
#
# Searches the files of the repository for content that does not belong in a
# public product repository. The checks form three groups:
#
#   content  home directories of real user accounts, private network
#            addresses, unfinished-work markers, identifiers and vocabulary of
#            the development process, Polish text, deployment patterns;
#   style    promotional and hedging wording, phrases addressed to the reader,
#            boilerplate headings, references to automated authorship tools,
#            emoji and decorative symbols, lint suppressions without a reason,
#            history comments, placeholder macros;
#   headers  source files without the producer header, per-file license
#            identifier lines (the license is stated in LICENSE), marks of the
#            authoring tool. Test data is not checked by this group: the
#            directories third_party/ and spec/golden/, and the directories
#            data/, corpus/, golden/ and fixtures/ below a tests/ directory at
#            any depth.
#
# Environment:
#   NURT_HYGIENE_PATTERNS          additional extended regular expressions, one
#                                  per line, matched case-insensitively
#   NURT_HYGIENE_PATTERNS_FILE     file with additional patterns, one per line
#   NURT_HYGIENE_REQUIRE_EXTERNAL  "1": fail when no additional pattern is given
#   NURT_HYGIENE_ALLOW_FILE        allow list (default tools/ci/hygiene-allow.txt)
#
# The allow list contains lines "<check> <glob>", where <check> is a check, a
# group or "*" for all checks. In a glob, "*" also matches "/". Lines starting
# with "#" are comments.
#
# Files are taken from "git ls-files" (tracked and untracked, not ignored), or
# from the directory tree when the directory is not a Git working tree.
#
# Patterns are written so that this file does not match them: bracket
# expressions split the words, and non-ASCII characters are produced as bytes.
#
# Exit status: 0 no findings, 1 findings, 2 usage or setup error.

set -eu
# Glob patterns are passed as data; pathname expansion is not used.
set -f

readonly GROUP_CONTENT="user-path private-address work-marker process-id process-word polish-text external-pattern"
readonly GROUP_STYLE="promotional-word hedging reader-address boilerplate tool-mention emoji allow-attribute history-comment placeholder-macro"
readonly GROUP_HEADERS="producer-header spdx-line tool-mark"
readonly CHECKS="$GROUP_CONTENT $GROUP_STYLE $GROUP_HEADERS"

usage() {
  cat <<'EOF'
Usage: repo-hygiene.sh [--only CHECKS | --skip CHECKS] | --list | --help

Checks the files of the repository for content that does not belong in a
public product repository. Without options, all checks are run.

  --only CHECKS   run only the listed checks or groups (comma-separated)
  --skip CHECKS   run all checks except the listed checks or groups
  --list          list the groups and checks
  --help          show this help

Groups: content, style, headers.
EOF
}

describe() {
  case "$1" in
    user-path) echo "home directories other than /home/user, /Users/user, C:/Users/user" ;;
    private-address) echo "IPv4 addresses of private networks (10/8, 172.16/12, 192.168/16)" ;;
    work-marker) echo "unfinished-work markers" ;;
    process-id) echo "identifiers of the form P-n, D-n, V-n" ;;
    process-word) echo "vocabulary of the development process" ;;
    polish-text) echo "Polish letters with diacritical marks" ;;
    external-pattern) echo "patterns from NURT_HYGIENE_PATTERNS or NURT_HYGIENE_PATTERNS_FILE" ;;
    promotional-word) echo "promotional and evaluative adjectives and adverbs" ;;
    hedging) echo "hedging phrases" ;;
    reader-address) echo "phrases addressed to the reader" ;;
    boilerplate) echo "boilerplate headings and introductions" ;;
    tool-mention) echo "references to automated authorship tools" ;;
    emoji) echo "emoji and decorative symbols" ;;
    allow-attribute) echo "lint suppressions in Rust without a reason" ;;
    history-comment) echo "comments about the history of the code" ;;
    placeholder-macro) echo "placeholder macros in Rust" ;;
    producer-header) echo "source files with comments that lack the producer header" ;;
    spdx-line) echo "per-file license identifier lines" ;;
    tool-mark) echo "marks of the authoring tool" ;;
  esac
}

group_of() {
  case " $GROUP_CONTENT " in *" $1 "*) echo content; return ;; esac
  case " $GROUP_STYLE " in *" $1 "*) echo style; return ;; esac
  echo headers
}

# Expands a comma-separated list of checks and groups into check names.
parse_checks() {
  result=""
  for name in $(printf '%s' "$1" | tr ',' ' '); do
    case "$name" in
      content) result="$result $GROUP_CONTENT" ;;
      style) result="$result $GROUP_STYLE" ;;
      headers) result="$result $GROUP_HEADERS" ;;
      *)
        case " $CHECKS " in
          *" $name "*) result="$result $name" ;;
          *)
            echo "repo-hygiene: unknown check '$name'" >&2
            exit 2
            ;;
        esac
        ;;
    esac
  done
  printf '%s' "$result"
}

selected=$CHECKS
case "${1:-}" in
  "") ;;
  --only)
    [ $# -eq 2 ] || { usage >&2; exit 2; }
    selected=$(parse_checks "$2")
    ;;
  --skip)
    [ $# -eq 2 ] || { usage >&2; exit 2; }
    skipped=" $(parse_checks "$2") "
    selected=""
    for check in $CHECKS; do
      case "$skipped" in
        *" $check "*) ;;
        *) selected="$selected $check" ;;
      esac
    done
    ;;
  --list)
    for check in $CHECKS; do
      printf '%-8s %-18s %s\n' "$(group_of "$check")" "$check" "$(describe "$check")"
    done
    exit 0
    ;;
  -h | --help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

export LC_ALL=C

if root=$(git rev-parse --show-toplevel 2>/dev/null); then
  in_git=1
else
  root=$(pwd)
  in_git=0
fi
cd "$root"

allow_file=${NURT_HYGIENE_ALLOW_FILE:-tools/ci/hygiene-allow.txt}
work=$(mktemp -d 2>/dev/null || mktemp -d -t nurt-hygiene)
trap 'rm -rf "$work"' EXIT
trap 'exit 2' INT TERM

findings=0
annotate=0
if [ "${GITHUB_ACTIONS:-}" = "true" ]; then
  annotate=1
fi

# Candidate files, one per line.
if [ "$in_git" -eq 1 ]; then
  git ls-files -z --cached --others --exclude-standard >"$work/all0"
  if [ "$(tr -cd '\n' <"$work/all0" | wc -c)" -ne 0 ]; then
    echo "repo-hygiene: file names containing a newline are not supported" >&2
    exit 2
  fi
  tr '\0' '\n' <"$work/all0" | sort -u >"$work/all.raw"
else
  find . -type d \( -name .git -o -name target \) -prune -o -type f -print |
    sed 's|^\./||' | sort -u >"$work/all.raw"
fi
: >"$work/all"
while IFS= read -r path; do
  if [ -f "$path" ] && [ ! -L "$path" ]; then
    printf '%s\n' "$path" >>"$work/all"
  fi
done <"$work/all.raw"

# Allow list entries: "<check> <glob>".
: >"$work/allow"
if [ -f "$allow_file" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      "" | "#"*) continue ;;
    esac
    printf '%s\n' "$line" >>"$work/allow"
  done <"$allow_file"
fi

is_allowed() {
  check=$1
  path=$2
  group=$(group_of "$check")
  while read -r entry_check entry_glob; do
    if [ "$entry_check" = "$check" ] || [ "$entry_check" = "$group" ] || [ "$entry_check" = "*" ]; then
      # shellcheck disable=SC2254
      case "$path" in
        $entry_glob) return 0 ;;
      esac
    fi
  done <"$work/allow"
  return 1
}

# Writes the files subject to a check to "$work/files", one per line.
select_files() {
  check=$1
  : >"$work/files"
  while IFS= read -r path; do
    if ! is_allowed "$check" "$path"; then
      printf '%s\n' "$path" >>"$work/files"
    fi
  done <"$work/all"
}

# Keeps the selected files whose name matches one of the given glob patterns.
keep_files() {
  : >"$work/files.kept"
  while IFS= read -r path; do
    for pattern in "$@"; do
      # shellcheck disable=SC2254
      case "$path" in
        $pattern)
          printf '%s\n' "$path" >>"$work/files.kept"
          break
          ;;
      esac
    done
  done <"$work/files"
  mv "$work/files.kept" "$work/files"
}

# Removes the selected files that are test data: inputs, expected results and
# corpora are kept byte for byte and carry no header.
drop_test_data() {
  : >"$work/files.kept"
  while IFS= read -r path; do
    case "$path" in
      third_party/* | */third_party/* | spec/golden/*) continue ;;
    esac
    case "/$path" in
      */tests/data/* | */tests/*/data/*) continue ;;
      */tests/corpus/* | */tests/*/corpus/*) continue ;;
      */tests/golden/* | */tests/*/golden/*) continue ;;
      */tests/fixtures/* | */tests/*/fixtures/*) continue ;;
    esac
    printf '%s\n' "$path" >>"$work/files.kept"
  done <"$work/files"
  mv "$work/files.kept" "$work/files"
}

# Source code formats with line comments.
readonly CODE_FILES="*.rs *.toml *.sh *.bash *.yml *.yaml *.py *.ps1 *.h *.c *.js *.ts"

# Escapes a value for a workflow command of GitHub Actions.
escape_annotation() {
  printf '%s' "$1" | sed -e 's/%/%25/g' -e 's/:/%3A/g' -e 's/,/%2C/g'
}

report() {
  check=$1
  file=$2
  line=$3
  text=$4
  findings=$((findings + 1))
  if [ "$annotate" -eq 1 ]; then
    printf '::error file=%s,line=%s,title=repo-hygiene %s::%s\n' \
      "$(escape_annotation "$file")" "$line" "$check" "$(printf '%s' "$text" | sed 's/%/%25/g')"
  else
    printf '%s: %s:%s: %s\n' "$check" "$file" "$line" "$text"
  fi
}

# Reports the records "file:line:text" of "$work/hits".
report_hits() {
  check=$1
  while IFS= read -r hit; do
    file=${hit%%:*}
    rest=${hit#*:}
    line=${rest%%:*}
    text=$(printf '%s' "${rest#*:}" | sed 's/^[[:space:]]*//' | cut -c1-160)
    report "$check" "$file" "$line" "$text"
  done <"$work/hits"
}

# Runs grep with the given options on the selected files and writes the
# matches to "$work/hits" as "file:line:text".
grep_files() {
  : >"$work/hits"
  grep_files_append "$@"
}

grep_files_append() {
  if [ -s "$work/files" ]; then
    tr '\n' '\0' <"$work/files" | xargs -0 grep -HnI "$@" -- >>"$work/hits" || true
  fi
}

# Removes the records of "$work/hits" that match an extended regular
# expression; further options for grep may precede the expression.
drop_hits() {
  grep -v "$@" "$work/hits" >"$work/hits.kept" || true
  mv "$work/hits.kept" "$work/hits"
}

# Home directories of user accounts. Only the placeholder account "user" is
# permitted, in examples. Trailing periods are punctuation, not part of a name.
check_user_path() {
  select_files user-path
  grep_files -oE '/home/[A-Za-z0-9._-]+|/Users/[A-Za-z0-9._-]+|[A-Za-z]:[\\/]+Users[\\/]+[A-Za-z0-9._-]+'
  drop_hits -E ':(/home/user|/Users/user|[A-Za-z]:[\\/]+Users[\\/]+user)\.*$'
  report_hits user-path
}

# Single IPv4 addresses of private networks. Network ranges in CIDR notation,
# for example 10.0.0.0/8, are permitted.
check_private_address() {
  select_files private-address
  octet='[0-9]{1,3}'
  grep_files -oE "(^|[^0-9.])(10\\.$octet\\.$octet\\.$octet|172\\.(1[6-9]|2[0-9]|3[01])\\.$octet\\.$octet|192\\.168\\.$octet\\.$octet)(/[0-9]{1,2})?([^0-9.]|$)"
  drop_hits -E '/[0-9]{1,2}[^0-9.]?$'
  report_hits private-address
}

check_work_marker() {
  select_files work-marker
  grep_files -wE 'T[O]DO|F[I]XME|X[X]X|H[A]CK'
  report_hits work-marker
}

check_process_id() {
  select_files process-id
  grep_files -wE '[PDV]-[0-9]+'
  report_hits process-id
}

check_process_word() {
  select_files process-word
  grep_files -iwE 'orch[e]strators?|sub-?[a]gents?|ork[i]estrator[a-z]*|pod[a]gent[a-z]*'
  report_hits process-word
}

# Polish letters with diacritical marks, as UTF-8 byte sequences.
check_polish_text() {
  select_files polish-text
  printf '\304\204\n\304\205\n\304\206\n\304\207\n\304\230\n\304\231\n' >"$work/polish"
  printf '\305\201\n\305\202\n\305\203\n\305\204\n\303\223\n\303\263\n' >>"$work/polish"
  printf '\305\232\n\305\233\n\305\271\n\305\272\n\305\273\n\305\274\n' >>"$work/polish"
  grep_files -F -f "$work/polish"
  report_hits polish-text
}

# Patterns supplied by the deployment. Only the location of a match is
# reported, so that the patterns are not repeated in the output.
check_external_pattern() {
  : >"$work/external"
  if [ -n "${NURT_HYGIENE_PATTERNS:-}" ]; then
    printf '%s\n' "$NURT_HYGIENE_PATTERNS" >>"$work/external"
  fi
  if [ -n "${NURT_HYGIENE_PATTERNS_FILE:-}" ]; then
    if [ ! -f "$NURT_HYGIENE_PATTERNS_FILE" ]; then
      echo "repo-hygiene: NURT_HYGIENE_PATTERNS_FILE does not name a file" >&2
      exit 2
    fi
    cat "$NURT_HYGIENE_PATTERNS_FILE" >>"$work/external"
  fi
  tr -d '\r' <"$work/external" | grep -vE '^[[:space:]]*(#|$)' >"$work/external.p" || true
  if [ ! -s "$work/external.p" ]; then
    if [ "${NURT_HYGIENE_REQUIRE_EXTERNAL:-0}" = "1" ]; then
      echo "repo-hygiene: no additional patterns supplied (NURT_HYGIENE_PATTERNS)" >&2
      exit 2
    fi
    echo "repo-hygiene: no additional patterns supplied; check external-pattern skipped" >&2
    return 0
  fi
  select_files external-pattern
  grep_files -iE -f "$work/external.p"
  while IFS= read -r hit; do
    file=${hit%%:*}
    rest=${hit#*:}
    report external-pattern "$file" "${rest%%:*}" "content matches an external pattern"
  done <"$work/hits"
}

# Promotional and evaluative words. The technical terms for file signatures
# and literal constants ("m[a]gic bytes", "m[a]gic number") are permitted.
check_promotional_word() {
  select_files promotional-word
  grep_files -iwE 'p[o]werful(ly)?|s[e]amless(ly)?|r[o]bust(ly|ness)?|b[l]azing(ly)?|e[l]egant(ly)?|e[f]fortless(ly)?|c[u]tting-edge|c[o]mprehensive(ly)?|s[i]mply|j[u]st|e[a]sily|m[a]gic(al|ally)?'
  drop_hits -iE 'm[a]gic (bytes?|numbers?|values?)'
  report_hits promotional-word
}

check_hedging() {
  select_files hedging
  grep_files -iwE 'should p[r]obably|might w[a]nt to|it s[e]ems'
  report_hits hedging
}

check_reader_address() {
  select_files reader-address
  apostrophe=$(printf "('|\342\200\231)")
  grep_files -iwE "l[e]t${apostrophe}s|h[e]re${apostrophe}s|h[a]ppy coding"
  report_hits reader-address
}

# Headings and introductions that repeat the content of a document.
check_boilerplate() {
  select_files boilerplate
  grep_files -iE '^#{1,6} +(o[v]erview|c[o]nclusion|k[e]y features)[[:space:]]*$|in this (module|file|section|document),? w[e] |this (module|file) c[o]ntains'
  report_hits boilerplate
}

readonly TOOL_MENTION_UPPER='A[I]|L[L]Ms?'
# The bare word agent is a technical term (SSH and gpg agents, HTTP client
# agents, the agent capability of the git protocol); only phrases about
# authorship by an agent are references.
readonly TOOL_MENTION_TERMS='artificial [i]ntelligence|language m[o]dels?|a[s]sistants?|chatb[o]ts?|system p[r]ompts?|prompt e[n]gineering|danaco a[g]ents?|(coding|autonomous|software) a[g]ents?|a[g]ents? (wrote|writes|generated|created|authored|produced)|(written|generated|created|authored|produced) (by|with) (an? |the )?a[g]ents?|as an a[g]ent'

# Permitted phrases of the tool-mention check, compared without regard to case:
# the product description, which names the users of the product in the
# repository description and in the first paragraph of README.md, the product
# terms in descriptions of functions and names of standards.
readonly TOOL_MENTION_ALLOWED='for people and A[I] a[g]ents|\bA[I] (coding )?a[g]ents?\b|model c[o]ntext protocol'

# Removes the permitted phrases from every hit and keeps the hits that still
# contain a reference, so that a permitted phrase does not hide another
# reference in the same line.
mask_tool_mentions() {
  sed -E "s/$TOOL_MENTION_ALLOWED//gI" "$work/hits" >"$work/hits.masked"
  {
    grep -nwE "$TOOL_MENTION_UPPER" "$work/hits.masked" || true
    grep -niwE "$TOOL_MENTION_TERMS" "$work/hits.masked" || true
  } | cut -d: -f1 | sort -un >"$work/hits.index"
  awk 'NR == FNR { keep[$1]; next } FNR in keep' "$work/hits.index" "$work/hits" >"$work/hits.kept"
  mv "$work/hits.kept" "$work/hits"
}

# References to automated authorship tools, except the phrases listed in
# TOOL_MENTION_ALLOWED. Identifiers such as user_agent do not match, because
# whole words are compared.
check_tool_mention() {
  select_files tool-mention
  grep_files -wE "$TOOL_MENTION_UPPER"
  grep_files_append -iwE "$TOOL_MENTION_TERMS"
  mask_tool_mentions
  sort -u "$work/hits" >"$work/hits.sorted"
  mv "$work/hits.sorted" "$work/hits"
  report_hits tool-mention
}

# Emoji (U+1F000 and above, variation selector 16) and decorative symbols
# (bullets, geometric shapes, miscellaneous symbols, dingbats, stars).
check_emoji() {
  select_files emoji
  pattern=$(printf '\360\237|\357\270\217|\342\200\242|\342[\226\227]|\342[\230-\236]|\342[\254\255]')
  grep_files -E "$pattern"
  report_hits emoji
}

# Lint suppressions in Rust need a reason: a "reason = ..." argument, a comment
# on the same line or a line comment (not a documentation comment) on the line
# before.
check_allow_attribute() {
  select_files allow-attribute
  keep_files '*.rs'
  : >"$work/hits"
  if [ -s "$work/files" ]; then
    # shellcheck disable=SC2016
    tr '\n' '\0' <"$work/files" | xargs -0 awk '
      FNR == 1 { prev = ""; open = 0 }
      {
        line = $0
        if (open) {
          if (line ~ /reason[[:space:]]*=/ || line ~ /\/\//) ok = 1
          if (line ~ /\)\]/) {
            open = 0
            if (!ok) print FILENAME ":" start ":" text
          }
          prev = line
          next
        }
        if (line ~ /#!?\[allow\(/) {
          ok = (line ~ /reason[[:space:]]*=/ || line ~ /\/\// || prev ~ /^[[:space:]]*\/\/([^\/!]|$)/)
          if (line ~ /\)\]/) {
            if (!ok) print FILENAME ":" FNR ":" line
          } else {
            open = 1
            start = FNR
            text = line
          }
        }
        prev = line
      }' >"$work/hits" || true
  fi
  report_hits allow-attribute
}

# Comments that describe the history of the code instead of its behaviour: a
# line comment that starts with "New:", "Updated:", "Fixed:", "(new)",
# "Changed to" or "Previously". Documentation comments (/// and //!) describe
# items such as enumeration variants and are not checked.
check_history_comment() {
  select_files history-comment
  # shellcheck disable=SC2086
  keep_files $CODE_FILES
  grep_files -iE '(^|[^/!])(//|#)[[:space:]]*((n[e]w|u[p]dated|f[i]xed)[[:space:]]*(:|$)|\(n[e]w\)|c[h]anged to|p[r]eviously)'
  report_hits history-comment
}

check_placeholder_macro() {
  select_files placeholder-macro
  keep_files '*.rs'
  grep_files -E 'tod[o]!\(|unimplemen[t]ed!\('
  report_hits placeholder-macro
}

# Source files in formats with comments start with the producer header, in the
# comment syntax of the format; its copyright line is required within the first
# lines. Files without comments (for example JSON) are not checked.
check_producer_header() {
  select_files producer-header
  drop_test_data
  # shellcheck disable=SC2086
  keep_files $CODE_FILES '*.md' .gitignore '*/.gitignore' .gitattributes '*/.gitattributes' .editorconfig
  while IFS= read -r path; do
    if ! sed -n '1,15p' "$path" |
      grep -qE 'Copyright \(c\) [0-9]{4} Danaco Official \| Danaco Holding Group Sp\. z o\.o\. All rights reserved\.'; then
      report producer-header "$path" 1 "missing producer header"
    fi
  done <"$work/files"
}

# Marks of the authoring tool are replaced by the producer header.
check_tool_mark() {
  select_files tool-mark
  drop_test_data
  grep_files -E 'Danaco A[g]ent'
  report_hits tool-mark
}

# The license is stated only in LICENSE; files carry no per-file license
# identifier.
check_spdx_line() {
  select_files spdx-line
  drop_test_data
  grep_files -E 'SPDX-License-Identifie[r]'
  report_hits spdx-line
}

for check in $selected; do
  "check_$(printf '%s' "$check" | tr '-' '_')"
done

total=$(wc -l <"$work/all" | tr -d ' ')
if [ "$findings" -ne 0 ]; then
  echo "repo-hygiene: $findings finding(s) in $total file(s)" >&2
  exit 1
fi
echo "repo-hygiene: no findings in $total file(s)"
