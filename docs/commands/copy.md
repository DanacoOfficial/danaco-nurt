---
command: copy
group: core
summary: Copy files and directories.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# copy

## Synopsis

```text
copy <from>... [<to>] (to: <path> | into: <path>) [name: value]...
entry |> copy [<to>] (to: <path> | into: <path>) [name: value]...
```

## Description

`copy` copies files, symbolic links, junctions and directory trees. The
source form takes the paths of `from`; the stage form takes `entry` records,
for example from `list` or `find`. `to` and `into` exclude each other, and
one of them is required.

`to` is the new path of the copy. When `to` names an existing directory, the
copy is placed inside it under the name of the source and the result reports
`to_is_dir`. `into` always names a directory that receives the copies; it is
created with its missing parents. Several sources need a directory: `into`,
or `to` naming an existing directory. `to` is evaluated per input record in
the stage form.

A directory is walked once, links not followed, and copied with its whole
content in one operation. An existing destination directory receives the
content of the source directory; conflicts are then decided per file. An
existing directory is never replaced by a file or a link, and a directory is
never copied over a file.

`on_conflict` decides what happens to an existing file or link at the
destination: `overwrite` replaces it, `skip` keeps it and leaves the source
out, `error` fails before anything changes. A symbolic link is copied as a
link with the same target; its target is not copied.

`preserve: true` keeps the permission bits of directories and lets the
journal keep the times, permissions and extended attributes of files where
the file system supports them. `clone: auto` uses block clones when source
and destination share a volume that supports them, and a byte copy
otherwise; `clone: always` fails when block clones are not available, and
`clone: never` always copies the bytes.

An entry record with `etag` must still have that entity tag; a changed entry
fails with `E_STALE` before anything changes. Empty input copies nothing and
reports `nothing_matched`.

| Notable code | Meaning |
|---|---|
| `to_is_dir` | `to` named an existing directory, which received the copy |
| `case_collision` | the destination differs from an existing name only in letter case on a volume that does not distinguish case |
| `nothing_matched` | the input had no entries |

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `from` | `list<path>` | required | Files and directories to copy in the source form. |
| `to` | `path` | one of `destination` | Target path; an existing directory receives the copy. |
| `into` | `path` | one of `destination` | Target directory, created when missing. |
| `on_conflict` | `enum(overwrite\|skip\|error)` | `overwrite` | Behaviour when the target file exists. |
| `preserve` | `bool` | `true` | Keep times, permissions and attributes. |
| `clone` | `enum(auto\|always\|never)` | `auto` | Use file system clones when available, always or never. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `source` | `none` | `effect` |
| `stage` | `entry` | `effect` |

The `effect` record has one `change` per created directory, copied file and
created link, with `from` set to the source. Its `summary` counts `files`,
`directories`, `links`, `overwritten` and `skipped`.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

## Examples

Copy a file next to itself:

```nurt example
copy "/srv/app/config.toml" "/srv/app/config.toml.bak"
```

```text expected
copy: 1 target, files 1
```

Copy a directory tree into a new directory:

```nurt example
copy "/srv/app/src" into: "/backup/2026-11-22"
```

Copy found log files and keep copies that already exist:

```nurt example
find "/srv/app/logs" glob: "*.log" |> copy into: "/archive/logs" on_conflict: skip
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | no source, or neither `to` nor `into` | give `from` and a destination |
| `E_PARAM_CONFLICT` | both `to` and `into`, or several sources and `to` is not an existing directory | give one destination; use `into` for several sources |
| `E_NOT_FOUND` | a source or the directory of `to` does not exist | check the path or use `into` |
| `E_EXISTS` | the target exists with `on_conflict: error`, is an existing directory, has another kind than the source, or two sources share one target | choose another destination or `on_conflict` value |
| `E_INVALID_VALUE` | a directory copied into itself, a source that is its own target, or `into` that is not a directory | choose a destination outside the source |
| `E_UNSUPPORTED` | `clone: always` without block clones, or a device, pipe or socket as a source | use `clone: auto`; exclude the special file |
| `E_STALE` | an input entry changed since it was read | read the entries again |
| `E_IN_USE` | Windows: a file is open in another process; the error lists the processes | close the file there |
| `E_PATH_RELATIVE` | a relative path without a base directory | give an absolute path or set `base` |

## Related

`move`, `rename`, `remove`, `create`, `find`, `list`.
