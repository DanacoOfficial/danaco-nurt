---
title: Journal and undo
order: 7
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Journal and undo

The journal records every operation that changes files, together with the previous content that undo needs. The contract is in the [specification](../../spec/journal.md).

## Commands

| Command | Function |
|---|---|
| `history` | lists operations; filters by state, path and range |
| `undo` | reverses one operation, the operations since a point, or the changes of given files |
| `redo` | applies an undone operation again |
| `journal.status` | shows whether the journal is on and what it holds |
| `journal.space` | shows free space and the space held by the journal |
| `journal.limits` | sets retention for the session or the user |
| `journal.pin` | keeps an entry beyond the retention limits |
| `journal.purge` | removes entries |

```nurt example
history limit: 5
```

## Entry states

| State | Meaning |
|---|---|
| `available` | the entry can be undone |
| `partial` | part of the preserved content was released to free disk space |
| `expired` | the entry left the retention window |
| `undone` | the entry was undone |

## Undo conflicts

When a file changed after the operation, `undo` reports `E_JOURNAL_CONFLICT` with the file and the hashes. The parameter `on_conflict` chooses `force`, `include_later`, `partial` or `abort`; the default `report` changes nothing.

```nurt example
undo 18 on_conflict: include_later
```

## Retention

The defaults keep the last 30 operations and the operations of the last 30 minutes. The keys `journal.max_ops`, `journal.max_age` and `journal.max_size` change the limits; see [Configuration](configuration.md). An operation whose previous content was not preserved is reported as `irreversible` with the reason.

## Crash safety

An operation on many targets is one journal entry. After a power loss the recovery step rolls back an entry that was not committed.
