#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Functions used by setup-nurt.sh. Compatible with bash 3.2 (macOS runners).

RELEASES_URL_DEFAULT='https://api.github.com/repos/DanacoOfficial/nurt/releases?per_page=100'
DOWNLOAD_BASE_DEFAULT='https://github.com/DanacoOfficial/nurt/releases/download'
SEMVER_RE='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

is_semver() {
  [[ ${1#v} =~ $SEMVER_RE ]]
}

# Sets SV_MAJOR, SV_MINOR, SV_PATCH and SV_PRE from a SemVer 2.0.0 string.
semver_split() {
  local v=${1#v}
  v=${v%%+*}
  SV_PRE=''
  case $v in
    *-*)
      SV_PRE=${v#*-}
      v=${v%%-*}
      ;;
  esac
  IFS=. read -r SV_MAJOR SV_MINOR SV_PATCH <<<"$v"
}

# Compares two identifier lists of a SemVer pre-release field.
# Prints -1, 0 or 1.
prerelease_cmp() {
  local a=$1 b=$2 ia ib
  while :; do
    if [[ -z $a && -z $b ]]; then
      echo 0
      return
    fi
    if [[ -z $a ]]; then
      echo -1
      return
    fi
    if [[ -z $b ]]; then
      echo 1
      return
    fi
    case $a in
      *.*) ia=${a%%.*}; a=${a#*.} ;;
      *) ia=$a; a='' ;;
    esac
    case $b in
      *.*) ib=${b%%.*}; b=${b#*.} ;;
      *) ib=$b; b='' ;;
    esac
    if [[ $ia =~ ^[0-9]+$ && $ib =~ ^[0-9]+$ ]]; then
      if ((10#$ia < 10#$ib)); then echo -1; return; fi
      if ((10#$ia > 10#$ib)); then echo 1; return; fi
    elif [[ $ia =~ ^[0-9]+$ ]]; then
      echo -1
      return
    elif [[ $ib =~ ^[0-9]+$ ]]; then
      echo 1
      return
    else
      if [[ $ia < $ib ]]; then echo -1; return; fi
      if [[ $ia > $ib ]]; then echo 1; return; fi
    fi
  done
}

# Prints -1, 0 or 1 for the precedence of version $1 relative to $2.
semver_cmp() {
  local a_major a_minor a_patch a_pre
  semver_split "$1"
  a_major=$SV_MAJOR a_minor=$SV_MINOR a_patch=$SV_PATCH a_pre=$SV_PRE
  semver_split "$2"
  if ((10#$a_major != 10#$SV_MAJOR)); then
    ((10#$a_major < 10#$SV_MAJOR)) && echo -1 || echo 1
    return
  fi
  if ((10#$a_minor != 10#$SV_MINOR)); then
    ((10#$a_minor < 10#$SV_MINOR)) && echo -1 || echo 1
    return
  fi
  if ((10#$a_patch != 10#$SV_PATCH)); then
    ((10#$a_patch < 10#$SV_PATCH)) && echo -1 || echo 1
    return
  fi
  if [[ -z $a_pre && -n $SV_PRE ]]; then
    echo 1
    return
  fi
  if [[ -n $a_pre && -z $SV_PRE ]]; then
    echo -1
    return
  fi
  prerelease_cmp "$a_pre" "$SV_PRE"
}

# Parses a possibly partial version (1, 1.2, 1.2.x, 1.2.3, 1.2.3-rc.1).
# Sets P_N (number of numeric parts, 0 to 3), P_MAJOR, P_MINOR, P_PATCH
# (missing parts are 0) and P_FULL (the normalised text of the lower bound).
partial_parse() {
  local t=${1#v} part idx=0
  P_N=0 P_MAJOR=0 P_MINOR=0 P_PATCH=0 P_FULL='0.0.0'
  case $t in
    '' | '*' | x | X) return 0 ;;
  esac
  if [[ $t =~ $SEMVER_RE ]]; then
    semver_split "$t"
    P_N=3 P_MAJOR=$((10#$SV_MAJOR)) P_MINOR=$((10#$SV_MINOR)) P_PATCH=$((10#$SV_PATCH))
    P_FULL=${t%%+*}
    return 0
  fi
  local rest=$t
  while [[ -n $rest ]]; do
    case $rest in
      *.*) part=${rest%%.*}; rest=${rest#*.} ;;
      *) part=$rest; rest='' ;;
    esac
    case $part in
      x | X | '*') break ;;
    esac
    [[ $part =~ ^[0-9]+$ ]] || return 1
    idx=$((idx + 1))
    case $idx in
      1) P_MAJOR=$((10#$part)) ;;
      2) P_MINOR=$((10#$part)) ;;
      3) P_PATCH=$((10#$part)) ;;
      *) return 1 ;;
    esac
    P_N=$idx
  done
  P_FULL="$P_MAJOR.$P_MINOR.$P_PATCH"
}

# Prints the first version after the range described by the partial version.
partial_next() {
  case $P_N in
    1) echo "$((P_MAJOR + 1)).0.0" ;;
    2) echo "$P_MAJOR.$((P_MINOR + 1)).0" ;;
    *) echo "$P_MAJOR.$P_MINOR.$((P_PATCH + 1))" ;;
  esac
}

# Prints the bounds of one comparator as "operator:version" lines.
# A comparator without bounds (any version) prints nothing.
expand_comparator() {
  local c=$1 op rest upper
  case $c in
    '>='*) op='>='; rest=${c#>=} ;;
    '<='*) op='<='; rest=${c#<=} ;;
    '>'*) op='>'; rest=${c#>} ;;
    '<'*) op='<'; rest=${c#<} ;;
    '='*) op='='; rest=${c#=} ;;
    '^'*) op='^'; rest=${c#^} ;;
    '~'*) op='~'; rest=${c#\~} ;;
    *) op='='; rest=$c ;;
  esac
  partial_parse "$rest" || return 1
  if ((P_N == 0)); then
    case $op in
      '>' | '<') return 1 ;;
    esac
    return 0
  fi
  case $op in
    '=')
      if ((P_N == 3)); then
        echo "=:$P_FULL"
      else
        echo ">=:$P_FULL"
        echo "<:$(partial_next)"
      fi
      ;;
    '>=') echo ">=:$P_FULL" ;;
    '>')
      if ((P_N == 3)); then echo ">:$P_FULL"; else echo ">=:$(partial_next)"; fi
      ;;
    '<') echo "<:$P_FULL" ;;
    '<=')
      if ((P_N == 3)); then echo "<=:$P_FULL"; else echo "<:$(partial_next)"; fi
      ;;
    '~')
      if ((P_N == 1)); then upper="$((P_MAJOR + 1)).0.0"; else upper="$P_MAJOR.$((P_MINOR + 1)).0"; fi
      echo ">=:$P_FULL"
      echo "<:$upper"
      ;;
    '^')
      if ((P_MAJOR > 0 || P_N == 1)); then
        upper="$((P_MAJOR + 1)).0.0"
      elif ((P_MINOR > 0 || P_N == 2)); then
        upper="0.$((P_MINOR + 1)).0"
      else
        upper="0.0.$((P_PATCH + 1))"
      fi
      echo ">=:$P_FULL"
      echo "<:$upper"
      ;;
  esac
}

# Succeeds when version $1 satisfies the range $2: alternatives separated by
# "||", each a space separated list of comparators that all must hold.
# A pre-release version satisfies a set only through an exact comparator.
range_satisfies() {
  local version=$1 range=$2 alt rest='' last=0 normalised
  normalised=$(printf '%s' "$range" | sed -E 's/(>=|<=|>|<|=|\^|~) +/\1/g')
  rest=$normalised
  while ((last == 0)); do
    case $rest in
      *'||'*)
        alt=${rest%%||*}
        rest=${rest#*||}
        ;;
      *)
        alt=$rest
        last=1
        ;;
    esac
    if set_satisfies "$version" "$alt"; then
      return 0
    fi
  done
  return 1
}

set_satisfies() {
  local version=$1 set=$2 c bounds line op bound result exact=0 is_pre=0
  semver_split "$version"
  [[ -n $SV_PRE ]] && is_pre=1
  set -f
  for c in $set; do
    if ! bounds=$(expand_comparator "$c"); then
      set +f
      return 1
    fi
    while IFS= read -r line; do
      [[ -n $line ]] || continue
      op=${line%%:*}
      bound=${line#*:}
      [[ $op == '=' ]] && exact=1
      result=$(semver_cmp "$version" "$bound")
      case $op in
        '=') [[ $result == 0 ]] || { set +f; return 1; } ;;
        '>=') [[ $result != -1 ]] || { set +f; return 1; } ;;
        '>') [[ $result == 1 ]] || { set +f; return 1; } ;;
        '<') [[ $result == -1 ]] || { set +f; return 1; } ;;
        '<=') [[ $result != 1 ]] || { set +f; return 1; } ;;
      esac
    done <<<"$bounds"
  done
  set +f
  if ((is_pre == 1 && exact == 0)); then
    return 1
  fi
  return 0
}

# Reads release tags from the JSON listing in file $1 and prints the SemVer
# versions without the leading "v", one per line.
extract_versions() {
  local tag
  grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' "$1" |
    sed -E 's/.*"([^"]*)"$/\1/' |
    while IFS= read -r tag; do
      if is_semver "$tag"; then
        printf '%s\n' "${tag#v}"
      fi
    done
}

# Prints the highest version in file $1 (one version per line) that
# satisfies the range $2. "latest" selects the highest stable version.
select_version() {
  local file=$1 spec=$2 best='' v
  while IFS= read -r v; do
    [[ -n $v ]] || continue
    if [[ $spec == latest ]]; then
      semver_split "$v"
      [[ -z $SV_PRE ]] || continue
    elif ! range_satisfies "$v" "$spec"; then
      continue
    fi
    if [[ -z $best || $(semver_cmp "$v" "$best") == 1 ]]; then
      best=$v
    fi
  done <"$file"
  [[ -n $best ]] || return 1
  printf '%s\n' "$best"
}

# Prints the version selected by the input $1. An exact SemVer is used
# without consulting the release listing.
resolve_version() {
  local spec=$1 listing versions version
  if is_semver "$spec"; then
    printf '%s\n' "${spec#v}"
    return 0
  fi
  listing=$(mktemp)
  versions=$(mktemp)
  if ! fetch "${NURT_RELEASES_URL:-$RELEASES_URL_DEFAULT}" "$listing"; then
    rm -f "$listing" "$versions"
    die "cannot download the release listing; check network access or pass an exact version"
  fi
  extract_versions "$listing" >"$versions"
  if version=$(select_version "$versions" "$spec"); then
    rm -f "$listing" "$versions"
    printf '%s\n' "$version"
    return 0
  fi
  rm -f "$listing" "$versions"
  die "no release satisfies version '$spec'"
}

# Prints the release target triple for an operating system ($1) and
# architecture ($2). Both accept runner names and uname output.
release_target() {
  local os arch
  case $1 in
    Linux | linux) os=linux ;;
    macOS | Darwin | darwin) os=macos ;;
    Windows | windows | MINGW* | MSYS* | CYGWIN*) os=windows ;;
    *) die "unsupported operating system '$1'" ;;
  esac
  case $2 in
    X64 | x64 | x86_64 | amd64 | AMD64) arch=x86_64 ;;
    ARM64 | arm64 | aarch64) arch=aarch64 ;;
    *) die "unsupported architecture '$2'" ;;
  esac
  case $os in
    linux) echo "$arch-unknown-linux-musl" ;;
    macos) echo "$arch-apple-darwin" ;;
    windows) echo "$arch-pc-windows-msvc" ;;
  esac
}

# Prints the tool-cache architecture name for an architecture ($1).
cache_arch() {
  case $1 in
    X64 | x64 | x86_64 | amd64 | AMD64) echo x64 ;;
    ARM64 | arm64 | aarch64) echo arm64 ;;
    *) die "unsupported architecture '$1'" ;;
  esac
}

release_base() {
  printf '%s\n' "${NURT_DOWNLOAD_BASE:-$DOWNLOAD_BASE_DEFAULT}"
}

archive_name() {
  local version=$1 target=$2
  case $target in
    *windows*) echo "nurt-$version-$target.zip" ;;
    *) echo "nurt-$version-$target.tar.gz" ;;
  esac
}

# Downloads URL $1 to file $2. file:// URLs are accepted.
fetch() {
  local url=$1 dest=$2
  if [[ -n ${GITHUB_TOKEN:-} && $url == https://api.github.com/* ]]; then
    curl --fail --silent --show-error --location --retry 3 \
      --header "Authorization: Bearer $GITHUB_TOKEN" --output "$dest" "$url"
  else
    curl --fail --silent --show-error --location --retry 3 --output "$dest" "$url"
  fi
}

# Prints the SHA-256 digest of file $1 in lower case.
sha256_of() {
  local out
  if command -v sha256sum >/dev/null 2>&1; then
    out=$(sha256sum "$1")
  elif command -v shasum >/dev/null 2>&1; then
    out=$(shasum -a 256 "$1")
  else
    die "sha256sum or shasum is required to verify downloads"
  fi
  printf '%s\n' "${out%% *}" | tr 'A-F' 'a-f'
}

# Checks file $1 against the entry for file name $2 in the checksum list $3
# (lines of "<digest>  <name>").
verify_checksum() {
  local file=$1 name=$2 sums=$3 expected actual
  expected=$(awk -v n="$name" '{ f = $2; sub(/^\*/, "", f); if (f == n) { print tolower($1); exit } }' "$sums")
  [[ -n $expected ]] || die "no checksum listed for $name"
  actual=$(sha256_of "$file")
  [[ $expected == "$actual" ]] ||
    die "checksum mismatch for $name: expected $expected, computed $actual"
}

# Extracts archive $1 into directory $2.
extract_archive() {
  local archive=$1 dest=$2
  case $archive in
    *.tar.gz) tar -xzf "$archive" -C "$dest" ;;
    *.zip)
      if command -v unzip >/dev/null 2>&1; then
        unzip -q -o "$archive" -d "$dest"
      elif command -v powershell.exe >/dev/null 2>&1; then
        powershell.exe -NoProfile -Command \
          "Expand-Archive -LiteralPath '$(cygpath -w "$archive")' -DestinationPath '$(cygpath -w "$dest")' -Force"
      else
        die "unzip is required to extract $archive"
      fi
      ;;
    *) die "unknown archive format: $archive" ;;
  esac
}
