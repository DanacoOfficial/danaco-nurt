---
command: move
group: core
summary: Move files and directories.
since: 0.1.0
---

# move

## Synopsis

```text
move <from>… <to> [on_conflict: overwrite|skip|error]
move <from>… into: <directory> [on_conflict: …]
<entry records> |> move <to> [on_conflict: …]
<entry records> |> move into: <directory> [on_conflict: …]
```

## Description

`move` moves files, symbolic links, junctions and directory trees to another
path. The source form takes the paths of `from`; the stage form takes
`entry` records. `to` and `into` exclude each other, and one of them is
required.

`to` is the new path of the entry. When `to` names an existing directory,
the entry is moved inside it under its own name and the result reports
`to_is_dir`. `into` always names a directory that receives the entries; it
is created with its missing parents. Several sources need a directory:
`into`, or `to` naming an existing directory. `to` is evaluated per input
record in the stage form.

Every entry is moved as a whole: a directory with its content, a link as a
link. Within one volume the entry is renamed. Between volumes the journal
copies the entry and removes the source after the copy is complete; the
result reports `cross_device`. A move that changes only the letter case of
the name on a volume that does not distinguish case renames the entry
itself.

`on_conflict` decides what happens to an existing file or link at the
destination: `overwrite` replaces it, `skip` keeps it and leaves the source
in place, `error` fails before anything changes. An existing directory is
never replaced. A directory cannot move into itself, and a source inside
another source of the same call is rejected.

An entry record with `etag` must still have that entity tag; a changed entry
fails with `E_STALE` before anything changes. Empty input moves nothing and
reports `nothing_matched`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `from` | `list<path>` | required in the source form | Files and directories to move. |
| `to` | `path` | | Target path; an existing directory receives the entry. Evaluated per record. |
| `into` | `path` | | Target directory, created when missing. |
| `on_conflict` | `enum(overwrite\|skip\|error)` | `overwrite` | Behaviour when the target file exists. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `move <from>… <to>` | none | `effect` |
| `… \|> move <to>` | `entry` | `effect` |

The `effect` record has one `change` with action `rename` per moved entry,
with `path` set to the source and `to` to the destination, and one `change`
per directory created for `into`. Its `summary` counts `moved`,
`overwritten`, `skipped` and `cross_device`.

## Effect

Category `change`, local effect, reversible. Every moved entry is a target of
the journal; an overwritten entry is kept by the journal, and undo moves the
entries back and restores it. All targets of one call are one operation.

## Notes

| Notable code | Meaning |
|---|---|
| `to_is_dir` | `to` named an existing directory, which received the entry |
| `cross_device` | the entry moved to another volume as a copy and a removal |
| `case_collision` | the destination differs from an existing name only in letter case on a volume that does not distinguish case |
| `nothing_matched` | the input had no entries |

## Examples

Move a file into an existing directory:

```nurt example
move "/home/user/report.txt" "/home/user/done"
```

```text expected
move: 1 target, moved 1, notable: to_is_dir
```

Move two files into a directory that does not exist yet:

```nurt example
move "/srv/a.log" "/srv/b.log" into: "/srv/old"
```

Move a file to another volume:

```nurt example
move "/data/export.csv" "/mnt/usb/export.csv"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | no source, or neither `to` nor `into` | give `from` and a destination |
| `E_PARAM_CONFLICT` | both `to` and `into`, or several sources and `to` is not an existing directory | give one destination; use `into` for several sources |
| `E_NOT_FOUND` | a source or the directory of `to` does not exist | check the path or use `into` |
| `E_EXISTS` | the target exists with `on_conflict: error`, is an existing directory, or two sources share one target | choose another destination or `on_conflict` value |
| `E_INVALID_VALUE` | a directory moved into itself, a source that is its own target, or a source inside another source | move only the enclosing directory |
| `E_STALE` | an input entry changed since it was read | read the entries again |
| `E_IN_USE` | Windows: an entry is open in another process; the error lists the processes | close the entry there |
| `E_PATH_RELATIVE` | a relative path without a base directory | give an absolute path or set `base` |

## Related

`rename`, `copy`, `remove`, `find`, `list`.
