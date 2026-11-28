---
category: git
title: Git
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/git.md -->
<!-- Responsibility:   Documents Git. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Git

## Show the status of a repository

Show the changed and untracked files of a repository.

```nurt example
git.status "/home/user/app"
```

```text expected
/home/user/app
```

```bash
git status --short
```

## Show the staged changes

Show the differences of the staged changes with their statistics.

```nurt example
git.diff "/home/user/app" staged: true stat: true
```

```text expected
files
```

## Show the history of a file

Show the last ten commits that changed a file.

```nurt example
git.log "/home/user/app" count: 10 path: ["src/main.go"]
```

```text expected
commit
```

## Show one commit

Show the changes of one revision.

```nurt example
git.show "/home/user/app" "HEAD"
```

```text expected
HEAD
```

## Find who changed a line

Show the author of each line of a range of a file.

```nurt example
git.blame "/home/user/app/src/main.go" lines: 10..20
```

```text expected
main.go
```

## Stage files and commit

Commit selected files with a message.

```nurt example
git.commit "/home/user/app" message: "fix: parser" paths: ["/home/user/app/a.go"]
```

```text expected
fix: parser
```

```bash
git add a.go && git commit -m "fix: parser"
```

## Commit with a trailer

Add a trailer line to the message of a commit.

```nurt example
git.commit "/home/user/app" message: "fix: parser" trailers: ["Co-Authored-By: Name <name@example.com>"] staged: true
```

```text expected
fix: parser
```

## Create and switch to a branch

Create a branch and switch to it.

```nurt example
git.switch "/home/user/app" branch: "feature/x" create: true
```

```text expected
feature/x
```

## Save a checkpoint before a risky change

Record the state of the working tree so that it can be restored.

```nurt example
git.checkpoint "/home/user/app" action: create label: "before refactoring"
```

```text expected
before refactoring
```

## Restore a checkpoint

Restore the working tree from a checkpoint.

```nurt example
git.checkpoint "/home/user/app" action: restore checkpoint_id: "before refactoring"
```

```text expected
before refactoring
```

## Stash the changes

Stash the changes, including untracked files.

```nurt example
git.stash "/home/user/app" action: push include_untracked: true
```

```text expected
stash
```

## Review the changes before a commit

Check the working tree for conflict markers, large files and secrets.

```nurt example
git.review "/home/user/app" scope: working checks: ["conflict_markers", "secrets"]
```

```text expected
finding
```

## Fetch from the remote

Fetch the branches and prune the deleted ones.

```nurt example
git.fetch "/home/user/app" prune: true
```

```text expected
origin
```

## Push a branch

Push a branch and set its upstream.

```nurt example
git.push "/home/user/app" branch: "feature/x" set_upstream: true
```

```text expected
feature/x
```

## Clone a repository

Clone a repository into a directory.

```nurt example
git.clone "https://example.com/team/app.git" "/home/user/src/app" depth: 1
```

```text expected
/home/user/src/app
```

## Show the untracked files

Show every untracked file, not only the collapsed directories, with the numbers of changed lines.

```nurt example
git.status "/home/user/app" untracked: all stat: true
```

```text expected
/home/user/app
```

```bash
git status --short --untracked-files=all
```

## Compare two revisions

Show the changes between two revisions of a path.

```nurt example
git.diff "/home/user/app" from: "v1.0.0" to: "HEAD" path: ["src"] stat: true
```

```text expected
files
```

```bash
git diff --stat v1.0.0 HEAD -- src
```

## Show a word-level diff

Show the changes of the working tree by words and ignore changes of whitespace.

```nurt example
git.diff "/home/user/app" word_diff: true ignore_whitespace: true
```

```text expected
hunk
```

## Show the commits of an author

Show the commits of one author since a date.

```nurt example
git.log "/home/user/app" author: "ann" since: "2026-10-01T00:00:00Z" count: 50
```

```text expected
commit
```

```bash
git log --author=ann --since=2026-10-01 -n 50
```

## Find the commit that introduced a text

Find the commits that added or removed a given text.

```nurt example
git.log "/home/user/app" pickaxe: "retryLimit" path: ["src"]
```

```text expected
commit
```

```bash
git log -S retryLimit -- src
```

## Group the commits by type

Group the last 100 commits by their conventional type.

```nurt example
git.log "/home/user/app" count: 100 group_by: type
```

```text expected
group
```

## Show a file in a revision

Show the changes of one file in a revision with its statistics.

```nurt example
git.show "/home/user/app" "HEAD~2" file: "/home/user/app/src/main.go" stat: true
```

```text expected
main.go
```

```bash
git show --stat HEAD~2 -- src/main.go
```

## List the branches

List the local branches of a repository.

```nurt example
git.branch "/home/user/app"
```

```text expected
main
```

```bash
git branch
```

## Create a branch from a revision

Create a branch that starts at a tag and keep the current branch.

```nurt example
git.branch "/home/user/app" create: "hotfix/1.0.1" from: "v1.0.0"
```

```text expected
hotfix/1.0.1
```

```bash
git branch hotfix/1.0.1 v1.0.0
```

## Delete a merged branch

Delete a branch; the repository keeps it in the trash so that it can be restored.

```nurt example
git.branch "/home/user/app" delete: "feature/x" trash: true
```

```text expected
feature/x
```

## Stage the changes of a path

Stage the changes of a directory.

```nurt example
git.stage "/home/user/app" action: add paths: ["/home/user/app/src"]
```

```text expected
src
```

```bash
git add src
```

## Unstage a file

Remove a file from the index and keep its changes.

```nurt example
git.stage "/home/user/app" action: unstage paths: ["/home/user/app/a.go"]
```

```text expected
a.go
```

```bash
git restore --staged a.go
```

## Restore a file from a revision

Restore a file to its state in the last commit and save a checkpoint first.

```nurt example
git.restore "/home/user/app" paths: ["/home/user/app/a.go"] source: "HEAD" checkpoint: true
```

```text expected
a.go
```

```bash
git restore --source=HEAD a.go
```

## Amend the last commit

Replace the message of the last commit.

```nurt example
git.commit "/home/user/app" message: "fix: parser error" amend: true
```

```text expected
fix: parser error
```

```bash
git commit --amend -m "fix: parser error"
```

## Create an annotated tag

Create an annotated tag for a release.

```nurt example
git.tag "/home/user/app" name: "v1.2.0" message: "release 1.2.0"
```

```text expected
v1.2.0
```

```bash
git tag -a v1.2.0 -m "release 1.2.0"
```

## Merge a branch

Merge a branch and refuse a merge that is not a fast-forward.

```nurt example
git.merge "/home/user/app" branch: "feature/x" fast_forward: only
```

```text expected
feature/x
```

## Abort a merge

Abort a merge that stopped on a conflict.

```nurt example
git.merge "/home/user/app" action: abort
```

```text expected
effect
```

```bash
git merge --abort
```

## Rebase onto a branch

Rebase the current branch onto the main branch.

```nurt example
git.rebase "/home/user/app" onto: "main"
```

```text expected
main
```

## Pull with a rebase

Fetch the upstream branch and rebase the local commits onto it.

```nurt example
git.pull "/home/user/app" mode: rebase
```

```text expected
origin
```

```bash
git pull --rebase
```

## Apply and drop a stash

Apply the newest stash and remove it from the stash list.

```nurt example
git.stash "/home/user/app" action: pop
```

```text expected
stash
```

## Add a worktree for a branch

Check out a branch in a second directory.

```nurt example
git.worktree "/home/user/app" action: add path: "/home/user/app-hotfix" branch: "hotfix/1.0.1"
```

```text expected
/home/user/app-hotfix
```

```bash
git worktree add /home/user/app-hotfix hotfix/1.0.1
```

## Push the tags

Push a branch together with the tags.

```nurt example
git.push "/home/user/app" branch: "main" tags: true
```

```text expected
main
```

## Push with a lease

Replace a remote branch only when it has not changed since it was fetched.

```nurt example
git.push "/home/user/app" branch: "feature/x" force: lease
```

```text expected
feature/x
```

```bash
git push --force-with-lease origin feature/x
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
