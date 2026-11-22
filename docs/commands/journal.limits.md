---
command: journal.limits
group: journal
summary: Show the active expiry limits, or change them and report how many entries will expire.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# journal.limits

## Synopsis

```text
journal.limits [steps: int | unlimited] [age: duration | unlimited] [size: size | percent | unlimited] [scope: session | user] [volume: path] [preview: bool]
```

## Description

`journal.limits` shows the limits that decide how long the journal keeps
entries, or changes them. Without `steps`, `age` and `size` the command only
reads: it reports the effective limits of every volume, or of the volume that
contains `volume`, together with the configuration level that set each of
them.

An entry expires when any of three limits is exceeded:

| Limit | Key | Meaning |
|---|---|---|
| `steps` | `journal.max_ops` | entry `n` expires when the number of later changing operations reaches the limit; a bulk operation, an undo and a redo count as one operation each |
| `age` | `journal.max_age` | entry expires when its age reaches the limit; the age is measured by `journal.age_clock` |
| `size` | `journal.max_size` | when the bytes held on a volume exceed the budget, the oldest entries expire until the bytes held fit |

`unlimited` switches a limit off. Free space for the user is protected in
every case: the journal never keeps data below the space reserve of a volume.

Each limit is resolved on its own from seven levels, in increasing
precedence: built-in defaults, the system file, the user file, the project
file `.nurt.toml` (only for a trusted project), the file named by
`NURT_CONFIG`, the `NURT_JOURNAL_*` environment variables and run-time
settings made by this command. `journal.volume."<mount>".*` takes precedence
over the general key of the same level.

A protection floor, `journal.floor.max_ops` and `journal.floor.max_age`, is
set by an administrator in the system file or by an embedding application.
A lower value from any level, including this command, is raised to the floor
and reported with `raised_to_floor`. Values from a project file are raised to
at least 10 steps and 5 minutes.

Lowering a limit takes effect at once: the entries over the new limit expire
during the call, oldest first. Raising a limit never brings back entries that
already expired. With `preview: true` the command reports which entries would
expire and changes nothing.

`scope: session` keeps the limits until the session of the request ends, or
until `session.ttl` passes without calls of the session. `scope: user` writes
the `[journal]` section of the user configuration file, keeping its comments
and layout; the change is recorded in the journal and does not count as a
step.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `steps` | `int \| enum(unlimited)` | | Maximum number of entries kept, `1` to `100000`. |
| `age` | `duration \| enum(unlimited)` | | Maximum age of entries kept, `10s` to `365d`. |
| `size` | `size \| percent \| enum(unlimited)` | | Maximum journal size as a size such as `20gib` (at least `1mib`, at most the capacity), a share of the volume such as `"10%"` (`1%` to `90%`) or a share of free space such as `"50%free"` (`1%free` to `100%free`). Shares are clamped to `journal.max_size_min` and `journal.max_size_max`. |
| `scope` | `enum(session\|user)` | `session` | Change the limits for this session or persistently for the account. |
| `volume` | `path` | | Only the journal of the volume that contains this path. |

## Input and output

| Input | Output |
|---|---|
| none | `journal_state`, `effect` |

The result lists the effective limits of each selected volume with their
sources, the numbers of the entries that expire (or would expire with
`preview`), their logical size in bytes, and whether the change was applied.

## Effect

| Category | Effect | Reversibility |
|---|---|---|
| journal | local | reversible |

The limits are reversed by setting them again. Entries that expired under the
new limits cannot be restored; the result then carries the irreversible
marker `journal_entries_expired`. Without parameters the command only reads.

## Examples

Show the limits of every volume and the levels that set them:

```nurt example
journal.limits
```

Report which entries would expire if only the last 10 operations were kept:

```nurt example
journal.limits steps: 10 preview: true
```

Keep the last 100 operations for one day in this session:

```nurt example
journal.limits steps: 100 age: 24h
```

Limit the journal on the volume of the project to a tenth of its capacity,
persistently for the account:

```nurt example
journal.limits size: "10%" scope: user volume: "/home/user/project"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | a value is out of its range, or a size budget of one volume is requested with `scope: user` | use a value within the range, or `scope: session` |
| `E_NOT_FOUND` | no journal volume contains `volume` | omit `volume` or name a path on a volume with a journal |
| `E_CONFIG` | the user configuration file is not valid | correct the file, then repeat the call |
| `E_IO` | a file of the journal or the configuration cannot be written | check the permissions of the state directory |

## Related

`journal.status`, `journal.purge`, `journal.pin`, `journal.space`.
