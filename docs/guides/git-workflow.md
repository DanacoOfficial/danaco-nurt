---
name: git-workflow
description: Working with git repositories through the built-in git.* commands.
commands: [git.status, git.diff, git.log, git.stage, git.commit, git.review, git.checkpoint, git.branch, git.switch, git.merge, git.push, git.fetch, undo]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/git-workflow.md -->
<!-- Responsibility:   Documents git-workflow. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# git-workflow

## Built-in operations

All operations of the group `git` are built in and work without an installed git: status, diff, log, show, blame, branch, stage, commit, tag, fetch, clone, switch, restore, reset, merge, rebase, stash, pull, push, worktree and checkpoint. `fetch`, `clone`, `pull` and `push` act outside the local file system and are recorded as irreversible. Authentication failures are reported as `E_GIT_AUTH` and unknown SSH hosts as `E_GIT_HOST_KEY`.

```nurt
git.status "/srv/app"
git.log "/srv/app" count: 5 path: ["/srv/app/a.go"]
```

## Reviewing before a commit

`git.diff` shows differences between revisions, the index and the working tree, with `staged`, `stat` and `symbol` narrowing the result. `git.review` checks changes before a commit for conflict markers, whitespace errors, large or binary files, secrets, missing tests and the format of the subject.

```nurt
git.diff "/srv/app" staged: true stat: true
git.review "/srv/app" scope: staged checks: [conflict_markers, secrets]
```

## Committing

`git.stage` stages files, hunks or lines, and `git.commit` records a commit with `message`, `paths` or `all`, and optional `trailers` and `sign_off`. A commit hook that fails stops the commit with `E_GIT_HOOK`. An empty commit fails with `E_GIT_NOTHING_TO_COMMIT` unless `allow_empty` is set.

```nurt
git.commit "/srv/app" message: "fix: parser" paths: ["/srv/app/a.go"]
```

## Checkpoints and undo

`git.checkpoint` keeps a snapshot of the working tree and index outside the branches, and `action: restore` returns to it. Operations that change local state, such as commit, switch, merge, rebase, reset and stash, are recorded in the journal; `undo` reverts a commit or a merge by moving the branch back to its previous revision.

```nurt
git.checkpoint "/srv/app" action: create label: "before-refactoring"
git.checkpoint "/srv/app" action: restore label: "before-refactoring"
```

## Branches and remotes

`git.branch` lists, creates, renames and deletes branches, and `git.switch` changes the working tree to a branch. A switch with local changes that would be overwritten fails with `E_GIT_DIRTY`. `git.push` with `force: lease` refuses to overwrite commits that were not fetched, and a push that the remote rejects fails with `E_GIT_PUSH_REJECTED`.

```nurt
git.switch "/srv/app" branch: "feature/parser" create: true
git.push "/srv/app" remote: "origin" branch: "feature/parser" set_upstream: true
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
