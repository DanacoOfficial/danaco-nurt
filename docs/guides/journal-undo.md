---
name: journal-undo
description: Repairing a mistake with the journal: history, undo, redo, limits and space.
commands: [history, undo, redo, journal.status, journal.space, journal.limits, journal.pin, journal.purge]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# journal-undo

## The journal

Every change to files, provider values or repository state is recorded with the previous state. `history` lists the operations, newest first, with their state: `available`, `partial`, `expired` or `undone`. `limit`, `since`, `before` and `path` narrow the list.

```nurt
history limit: 10
history path: "/srv/app/main.go"
```

## Undoing and redoing

`undo` reverts the last operation, or the operation named by `id`. `to` and `since` revert several operations, newest first, and `paths` restores only the named paths of an operation. `redo` applies an undone operation again. Undo repairs a mistake that is visible in the result; checking with `preview: true` or `assert` before the change is still the safer course.

```nurt
undo paths: ["/srv/app/main.go"]
redo
```

## Conflicts

When a file changed after the operation, undoing it would discard the later change. `on_conflict` selects the behavior: `report` stops and lists the conflicts, `abort` leaves everything as it was, `force` restores the old state, `include_later` also reverts the later operations and `partial` restores the paths without a conflict. The error `E_JOURNAL_CONFLICT` lists the conflicting paths.

```nurt
undo on_conflict: partial
```

## Partial and irreversible entries

An entry is `partial` when the previous state of some targets could not be kept. `history state: partial` lists such entries, and `undo on_conflict: partial` restores what was kept. Effects outside the file system, such as the start of a service or a push to a remote, are recorded as irreversible and are reported separately. The error `E_JOURNAL_PARTIAL` marks an undo that restored only part of an operation.

## Limits and space

Entries expire when the limits of the journal are exceeded. `journal.status` shows the number of entries, the space used, the oldest entry and the active limits. `journal.limits` shows or changes the limits, `journal.pin` protects entries from expiry and `journal.space` reports the space that the journal holds on each volume. `journal.purge` releases entries and the space they hold, and cannot be undone.

```nurt
journal.status
journal.limits scope: user
journal.space
```
