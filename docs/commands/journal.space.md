---
command: journal.space
group: journal
summary: Report free space, space reclaimable from the journal and the reserve of each volume.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# journal.space

## Synopsis

```text
journal.space [volume: path]
```

## Description

Reports one `volume` record per journal volume: free space as other programs
see it, the space the journal holds and could release, and the thresholds that
protect user space. Without `volume` the command reports every volume known to
the space service; with `volume` it reports the volume that contains the path.

Free space `F` is the space available to the account after the journal took
its share and after the account quota, when quotas apply (`statvfs` on Linux,
`volumeAvailableCapacityKey` on macOS, `GetDiskFreeSpaceExW` on Windows).
Purgeable space reported by macOS is not counted.

The thresholds are derived from the capacity `T` of the volume, or from the
account quota when that is smaller:

| Threshold | Key | Default |
|---|---|---|
| Reserve `R` | `space.reserve` | 15 % of `T`, clamped to `space.reserve_min` (5 GiB) and `space.reserve_max` (50 GiB); with an account quota no more than 15 % of the quota |
| Soft threshold `S` | `space.soft` | `R` plus the larger of 5 % of `T` and 2 GiB, no more than `R + 20 GiB`; always above `R` |
| Margin `m` | `space.margin` | the smaller of 1 % of `T` and 1 GiB |
| Hysteresis `h` | none | 1 % of `T` |

A 1 TB disk has `R` = 50 GiB and `S` = 70 GiB; a 32 GB disk has `R` = 5 GiB and
`S` = 7 GiB.

The thresholds define the state of the volume:

| State | Condition | Journal behaviour |
|---|---|---|
| normal | `F ≥ S` | previous content is kept; entries expire by their limits only |
| soft | `R ≤ F < S` | previous content is kept only when `F − extra ≥ R + m` after the write; the service releases old entries in the background until `F ≥ S + h` |
| hard | `F < R` | no previous content is kept (`no_space_for_copy`); the service releases entries at once until `F ≥ R + m`, large entries in part, largest files first |
| emergency | `F < R / 2` | as hard, with release on every processor core; commands do not wait for the service |

Before a change the journal keeps previous content of a target only when the
volume is eligible, the space service is running, the operation fits
`journal.max_op_size` and `journal.max_size`, and `F − extra ≥ R + m` and
`bytes_held + extra ≤ journal.max_size` both hold after the write, where
`extra` is the space the journal additionally occupies. Otherwise expired
entries and then old entries are released within `space.sync_budget_ms`
(1500 ms); when that is not enough, the target is changed without preservation
and the result marks it irreversible. Entries released for space are chosen by
bytes released per second of removal; pinned entries are released for space
like any other entry.

Guarantees:

- The journal never causes a user operation to fail for lack of space.
- While the space service is not running, the journal keeps no new data.
- The journal itself never lowers free space below the reserve; other
  programs can.
- A write that needs no more than `F` at its start, while `F ≥ R`, succeeds;
  in particular every write of at most `R` bytes.

Not guaranteed:

- A write that needs more than `F` but no more than `F` plus the reclaimable
  bytes succeeds only when the service releases entries before the write runs
  out of space; installers that check free space in advance see `F` only.
- After a write lowers `F` below `R`, free space returns above `R` after the
  release completes; for entries with millions of files this takes minutes.
- An external write that is larger than `F` and faster than release can run
  out of space; the reserve protects writes up to `R`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `volume` | `path` | none | Only the volume that contains this path. |

## Input and output

Input: none. Output: `volume` records with the fields:

| Field | Type | Meaning |
|---|---|---|
| `mount` | `path` | mount point or volume root |
| `filesystem` | `string` | file system type, for example `ext4`, `apfs`, `ntfs` |
| `total` | `size` | capacity, or the account quota when smaller |
| `free` | `size` | free space `F` as other programs see it |
| `free_without_journal` | `size` | free space if the journal held nothing |
| `reclaimable` | `size` | space the journal could release now |
| `reserve` | `size` | reserve `R` |
| `soft` | `size` | soft threshold `S` |
| `journal_bytes` | `size` | logical size of the data the journal holds |
| `inactive` | `string` | why the journal keeps no data on the volume: `service_down`, `network_volume`, `removable_volume`, `memory_volume`, `below_reserve`, `no_anchor`, `disabled` or `not_durable`; absent when the journal is on |

Sizes are logical: data shared with names outside the journal through hard
links counts as zero, and on file systems that release blocks lazily `free`
grows up to a few seconds after the release.

## Effect

Category `journal`, effect `read`, reversibility `read`. The command changes
nothing and uses no journal entry number.

## Examples

Report free space and the journal share of every journal volume:

```nurt example
journal.space
```

Report the volume of the home directory:

```nurt example
journal.space volume: "/home/user"
```

Report the volume that contains a project directory; the path selects its
volume and need not be the mount point:

```nurt example
journal.space volume: "/home/user/projects/app"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | no volume contains the path | give an existing path or omit `volume` |
| `E_IO` | free space or the journal index cannot be read | check the volume and repeat the command |

## Related

`journal.status` reports entries, limits and their sources; `journal.purge`
releases entries at once; `journal.limits` changes the step, age and size
limits.
