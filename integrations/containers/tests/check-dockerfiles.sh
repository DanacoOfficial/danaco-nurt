#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
# Static checks of the image definitions: labels, license files, build
# arguments, absence of compilation steps and the non-root user.

set -uo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/.." && pwd)
failures=0

require() {
  local file=$1 pattern=$2 description=$3
  if grep -Eq -- "$pattern" "$file"; then
    printf 'ok   %s: %s\n' "$file" "$description"
  else
    printf 'FAIL %s: %s\n' "$file" "$description"
    failures=$((failures + 1))
  fi
}

forbid() {
  local file=$1 pattern=$2 description=$3
  if grep -Eq -- "$pattern" "$file"; then
    printf 'FAIL %s: %s\n' "$file" "$description"
    failures=$((failures + 1))
  else
    printf 'ok   %s: %s\n' "$file" "$description"
  fi
}

for name in nurt ci devcontainer; do
  file="$root/$name/Dockerfile"
  [[ -f $file ]] || { printf 'FAIL %s: missing\n' "$file"; failures=$((failures + 1)); continue; }
  require "$file" 'org\.opencontainers\.image\.source="https://github\.com/DanacoOfficial/danaco-nurt"' 'source label'
  require "$file" 'org\.opencontainers\.image\.licenses="Apache-2\.0"' 'license label'
  require "$file" 'org\.opencontainers\.image\.vendor="Danaco Holding Group"' 'vendor label'
  require "$file" 'COPY .*LICENSE NOTICE /usr/share/doc/nurt/' 'LICENSE and NOTICE copied'
  require "$file" 'COPY .*\$\{TARGETARCH\}/nurt /usr/local/bin/nurt' 'executable copied from the build context'
  require "$file" '^ENV NURT_STATE_DIR=/var/lib/nurt$' 'state directory variable'
  forbid "$file" 'cargo|rustc|rustup' 'no compilation in the image'
  forbid "$file" ':latest' 'no floating tag'
done

require "$root/nurt/Dockerfile" '^FROM scratch$' 'final stage is scratch'
require "$root/nurt/Dockerfile" '^USER [0-9]+:[0-9]+$' 'numeric non-root user'
require "$root/nurt/Dockerfile" '^ENTRYPOINT \["/usr/local/bin/nurt"\]$' 'entry point'
require "$root/ci/Dockerfile" '^USER [0-9]+:[0-9]+$' 'numeric non-root user'
require "$root/ci/Dockerfile" 'install -y --no-install-recommends ca-certificates git' 'git and CA certificates'
require "$root/devcontainer/Dockerfile" 'devcontainer\.metadata=' 'devcontainer metadata'
require "$root/devcontainer/Dockerfile" '"danaco\.nurt"' 'extension recommendation'

((failures == 0))
