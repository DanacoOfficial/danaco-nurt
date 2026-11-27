---
command: journal.pin
group: journal
summary: Protect journal entries from expiry by steps, age and size, or release that protection.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/journal.pin.md -->
<!-- Responsibility:   Documents journal.pin. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# journal.pin

## Synopsis

```text
journal.pin <id>... [name: value]...
```

## Description

`journal.pin` protects journal entries from expiry by the step limit, the age
limit and the size budget for the given duration. A pinned entry stays
undoable after newer operations, time and other entries would have expired
it. Pinning never protects an entry from reclaim when the volume needs space
for the user: free space for the user always has priority.

Without `until` the pin lasts `journal.pin_max_age` (24 hours by default).
Pinning a pinned entry again replaces its pin. `release: true` removes the
pins; released entries that are over a limit at that moment expire at once
and are listed in `expired`.

The pin is recorded outside the entry, in the journal directory of every
volume that holds a part of the operation. Pinning changes neither the entry
nor its hash, creates no journal entry and consumes no operation number and
no step.

Entries that do not exist, expired or were undone are listed in
`not_available` and left alone. The result does not state when a pin ends;
`history` shows whether an operation is pinned.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `id` | `list<int>` | none | Entries to pin or release. |
| `release` | `bool` | `false` | Release the pin instead of setting it. |
| `until` | `duration` | the configured maximum pin age | Duration of the pin. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `journal` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |
| Reversibility conditions | Pinned entries can still be released when the volume runs out of space. |

## Examples

Keep operation 18 undoable for the default duration:

```nurt example
journal.pin 18
```

Pin two operations for three days:

```nurt example
journal.pin 18 19 until: 3d
```

Release the pin of operation 18:

```nurt example
journal.pin 18 release: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_IO` | the pin file of a volume cannot be written, or the journal of a volume of the entry is not available | mount the volume and check its journal with `journal.status` |

## Related

`history`, `journal.status`, `journal.limits`, `journal.purge`, `undo`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
