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
journal.limits [name: value]...
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
already expired. With the common parameter `preview: true` the command
reports the effective limits and the entries that would expire, and changes
nothing.

`scope: session` keeps the limits until the session of the request ends, or
until `session.ttl` passes without calls of the session. `scope: user` writes
the `[journal]` section of the user configuration file, or the
`[journal.volume."<mount>"]` table with `volume`, keeping its comments and
layout. Missing directories of the file are created. The change is recorded
as a journal entry of kind `journal.limits` that does not count as a step;
`undo` of that entry restores the previous content of the file. `scope: user`
requires the location of the user configuration file to be configured for
the journal; otherwise a call that sets a limit fails with `E_UNSUPPORTED`,
and a call without `steps`, `age` and `size` only reports.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `steps` | `int \| enum(unlimited)` | none | Maximum number of entries kept. |
| `age` | `duration \| enum(unlimited)` | none | Maximum age of entries kept. |
| `size` | `size \| percent \| enum(unlimited)` | none | Maximum journal size as a size such as 20gib, a share of the volume such as "10%" or a share of free space such as "50%free". |
| `scope` | `enum(session\|user)` | `session` | Change the limits for this session or persistently for the account. |
| `volume` | `path` | none | Only the journal of the volume that contains this path. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `journal_state`, `effect` |

Without `steps`, `age` and `size` the output is one `journal_state` record per
selected volume, as from `journal.status`. Otherwise the output is one
`effect` record of kind `journal.limits`: `after.applied` states whether the
new limits were set, `after.expiring` lists the entries that expire (or would
expire with `preview`), `after.expiring_bytes` gives their logical size, and
`after.volumes` lists the effective limits of each selected volume with their
sources.

## Effect

| Property | Value |
|---|---|
| Category | `journal` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |
| Reversibility conditions | Restoring the previous limits does not bring back entries that expired under the new limits; without parameters the command only reads. |

The limits are reversed by setting them again. Entries that expired under the
new limits cannot be restored; the result then carries the irreversible
marker `journal_entries_expired`. Without parameters the command only reads.

## Examples

Show the limits of every volume and the levels that set them:

```nurt example
journal.limits
```

Keep the last 10 operations for five minutes in this session:

```nurt example
journal.limits steps: 10 age: 5m
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
| `E_INVALID_VALUE` | a value is out of its range, or `scope: session` is used by a request without a session | use a value within the range; set the request option `session` or use `scope: user` |
| `E_NOT_FOUND` | no journal volume contains `volume` | omit `volume` or name a path on a volume with a journal |
| `E_CONFIG` | the user configuration file is not valid TOML or UTF-8 | correct the file, then repeat the call |
| `E_UNSUPPORTED` | `scope: user` and no user configuration file is configured for the journal | configure the location of the user configuration file or use `scope: session` |
| `E_IO` | a file of the journal or the configuration cannot be written | check the permissions of the state directory |

## Related

`journal.status`, `journal.purge`, `journal.pin`, `journal.space`.
