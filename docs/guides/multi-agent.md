---
name: multi-agent
description: Several agents or sessions changing the same files without overwriting each other.
commands: [read, info, write, replace, edit, set, history, undo]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/multi-agent.md -->
<!-- Responsibility:   Documents multi-agent. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# multi-agent

## The etag

Every file read carries an `etag`, a value that identifies its content. A change to the file is applied only when the file still has the content that was read. `read` and `info` return the `etag` in their records, and a changing command checks it before it writes.

```nurt
info "/srv/app/config.toml"
```

## Conditional writes

The parameter `if_match` makes the check explicit: the change is applied only when the file has the given `etag`. `write`, `replace`, `insert`, `edit`, `set` and `patch` accept it. Without `if_match` the check uses the state that the same call read.

```nurt
write "/srv/app/config.toml" "mode = 1\n" if_match: "a41f9c"
```

## Stale content

A change to a file that another session modified after it was read fails with `E_STALE` and leaves the file unchanged. The error names the path and the current `etag`. The remedy is to read the file again, decide whether the change still applies and send the change with the new `etag`.

## Retrying

A read that follows `E_STALE` returns the current content. `read` with `unchanged_since` returns nothing when the file did not change since the given `etag`, which makes a repeated check cheap. A program that changes several files stops at the first conflict before any file is written.

```nurt
read "/srv/app/config.toml" unchanged_since: "a41f9c"
```

## Sessions and the journal

The call parameter `session` names the session of a call. The journal records the session of each operation, and `history` lists the operations of all sessions; `undo` reverts only the operations of the current session unless `all_sessions: true` is given. An undo of an operation whose files were changed by another session reports the conflict instead of discarding the other change; see the guide `journal-undo`.

```nurt
history limit: 20
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
