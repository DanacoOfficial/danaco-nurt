---
category: git
title: Git
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
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
