#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           website/scripts/build-at-tag.sh
# Responsibility:   Builds the site of a version with the repository at the tag of that version.
# Dependencies:     git, node, npm, the installed dependencies of website/ (node_modules)
# Author:           Danaco Official
# Version:          0.1.2, 2026-11-28
# License:          see the LICENSE file or https://www.danaco-group.pl

#
# The site reads the documentation, the specification, the schemas, the
# installers and the counted numbers from the directories beside website/.
# This script builds the directory website/ of one revision on the tree of a
# tag, in a temporary worktree, runs the checks of the package and copies the
# result to the output directory. The repository and its branches stay as they
# are; the worktree is removed at the end.
#
# Usage: website/scripts/build-at-tag.sh <tag> <output-directory> [<website-revision>]
# Example: website/scripts/build-at-tag.sh v0.1.2 /srv/site/releases/2026-11-28-0.1.2-34
#
# The website revision defaults to HEAD. The output directory must not exist.
# The file RELEASE in the output directory names the version, the date, both
# revisions and the address of the site.
#
# Exit codes: 0 built and checked, 1 the build or a check failed, 2 wrong use.

set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
  echo "usage: build-at-tag.sh <tag> <output-directory> [<website-revision>]" >&2
  exit 2
fi

tag=$1
output=$2
revision=${3:-HEAD}

if [ -e "$output" ]; then
  echo "build-at-tag: the output directory exists: $output" >&2
  exit 2
fi

website=$(cd "$(dirname "$0")/.." && pwd)
repository=$(git -C "$website" rev-parse --show-toplevel)
tag_revision=$(git -C "$repository" rev-parse --short=9 "$tag^{commit}")
website_revision=$(git -C "$repository" rev-parse --short=9 "$revision^{commit}")
branch=$(git -C "$repository" rev-parse --abbrev-ref HEAD)

if [ ! -d "$website/node_modules" ]; then
  echo "build-at-tag: install the dependencies first: npm ci in $website" >&2
  exit 2
fi

worktree=$(mktemp -d "${TMPDIR:-/tmp}/nurt-site-at-tag.XXXXXX")
cleanup() {
  rm -f "$worktree/website/node_modules"
  git -C "$repository" worktree remove --force "$worktree" >/dev/null 2>&1 || rm -rf "$worktree"
}
trap cleanup EXIT

rmdir "$worktree"
git -C "$repository" worktree add --detach "$worktree" "$tag_revision" >/dev/null
git -C "$worktree" checkout "$website_revision" -- website
ln -s "$website/node_modules" "$worktree/website/node_modules"

(
  cd "$worktree/website"
  npm run build
  npm run check
)

mkdir -p "$(dirname "$output")"
cp -a "$worktree/website/dist" "$output"
version=$(node -p "require('$worktree/website/package.json').version")
{
  echo "version=$version"
  echo "built=$(date -u +%Y-%m-%d)"
  echo "source=website (branch $branch, revision $website_revision) built with the repository at the tag $tag (revision $tag_revision)"
  echo "site=${SITE_URL:-https://nurt.danaco-group.pl}/"
} > "$output/RELEASE"

echo "build-at-tag: $output holds the site of $website_revision on $tag ($tag_revision)"
