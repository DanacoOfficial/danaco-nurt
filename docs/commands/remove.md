---
command: remove
group: core
summary: Remove files, directories, registry keys and values, lines, matches, document items, syntax nodes or finished jobs.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# remove

## Synopsis

```text
remove <path>... [name: value]...
entry|registry_value|line|match|item|node|job |> remove [name: value]...
```

## Description

`remove` removes files, symbolic links, junctions and directories given by
path or by `entry` records. A directory is removed with its whole content as
one target, without a separate confirmation. A link is removed as a link;
its target stays. A path inside another removed path is removed once, with
the enclosing directory.

`on_missing` decides what happens to a path that does not exist: `skip`
counts it as `missing` and continues, `error` fails with `E_NOT_FOUND`
before anything changes. An `entry` record with `exists: false`, as `info`
produces it, is a missing path.

`line` records, as `lines` and `read` produce them, remove whole lines with
their line endings; `match` records, as `search` produces them, remove the
matched text at its line and column. All lines and matches of one file are
removed by one write that keeps the encoding, the byte order mark and the
line endings of the file. A file cannot be removed and edited in the same
call. `item`, `node` and `job` records are removed by the stages of the
document, syntax and job commands.

`check_references: true` searches the files below the base directory, or
below the directory of the first removed path, for whole-word occurrences of
the removed names and of the names without their last extension, and
reports them as `references_left` with up to 20 locations. Hidden entries,
version control, dependency, build and cache directories, binary files and
files larger than 1 MiB are not searched.

A record with `etag` must still have that entity tag; a changed entry or
file fails with `E_STALE` before anything changes. When nothing is removed,
the result reports `nothing_matched`.

| Notable code | Meaning |
|---|---|
| `references_left` | other files still name a removed entry; `detail` lists the locations |
| `nothing_matched` | nothing was removed |

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `list<path>` | required | Paths to remove in the source form; directories are removed recursively. |
| `on_missing` | `enum(skip\|error)` | `skip` | Behaviour when a path does not exist. |
| `check_references` | `bool` | `false` | Report references to the removed names left after the change. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `source` | `none` | `effect` |
| `stage` | `entry`, `registry_value`, `line`, `match`, `item`, `node`, `job` | `effect` |

The `effect` record has one `change` with action `delete` per removed entry;
a directory removed with its content has the flag `recursive`. A file edited
by `line` or `match` records has one `change` with action `modify` and the
number of removed lines or matches. The `summary` counts `removed`, `files`,
`directories`, `links`, `bytes`, `missing`, `lines` and `matches`.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |
| Reversibility conditions | Removed files, directories and content are kept in the journal. |

| Action | Effect | Reversibility | Note |
|---|---|---|---|
| `job` | `local` | `irreversible` | Removing a job deletes its record and output files without a journal copy. |

## Examples

Remove a file:

```nurt example
remove "/home/user/build.log"
```

```text expected
remove: 1 target, files 1
```

Remove a directory tree and ignore a path that does not exist:

```nurt example
remove "/home/user/app/cache" "/home/user/app/old.log"
```

Remove two lines from a file:

```nurt example
lines "/home/user/notes.txt" 2..3 |> remove
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | the source form has no path | give `path` |
| `E_NOT_FOUND` | a path does not exist with `on_missing: error` | check the path or use `on_missing: skip` |
| `E_STALE` | an entry or file changed since it was read, or a line or match is no longer where the record says | read or search the file again |
| `E_INVALID_VALUE` | a file is removed and edited in the same call | remove the file or its lines, not both |
| `E_TYPE` | a record of a kind `remove` does not accept, or a `line` or `match` record without its number fields | pass records of `list`, `find`, `lines`, `read` or `search` |
| `E_IN_USE` | Windows: an entry is open in another process; the error lists the processes | close the entry there |
| `E_PATH_RELATIVE` | a relative path without a base directory | give an absolute path or set `base` |

## Related

`move`, `rename`, `copy`, `lines`, `search`, `find`, `undo`.
