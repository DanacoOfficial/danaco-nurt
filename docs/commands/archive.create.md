---
command: archive.create
group: archive
summary: Create an archive from the input entries.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# archive.create

## Synopsis

```text
entry |> archive.create <to> [name: value]...
```

## Description

`archive.create` writes a new archive `to` with the files, directories and
links named by the input `entry` records, for example the records of `find`
or `list`. A directory adds its whole tree. An entry given more than once,
such as a directory and the files `find` lists inside it, is stored once.

The name of a member is the path of the entry relative to `base`. Without
`base` it is the `relative` field of the record, which `find` and `list`
set, otherwise the name of the entry. An entry outside `base` is an error;
the entry `base` itself adds its content without a member of its own.

Members are stored in byte order of their names, with the permissions and
modification times of the files and without account names, so the same tree
always produces the same archive. Symbolic links are stored as links; with
`follow_links: true` the files and directories they lead to are stored
instead, and a link that leads back into a directory being stored is skipped
and reported with `link_loop_skipped`. Devices, FIFOs and other special
files are skipped and reported with `special_file`.

`format: auto` selects the format from the extension of `to`: `.zip`,
`.jar`, `.whl`, `.apk`, `.tar`, `.tar.gz` or `.tgz`, `.tar.zst` or `.tzst`,
`.tar.xz` or `.txz`, `.tar.bz2` or `.tbz2`, `.7z`. `level` is the compression
level of the format: 0 to 9 for zip (0 stores members uncompressed) and 7z,
and the range of the stream algorithm for compressed tar (gzip, xz and bzip2
0 to 9, zstd 1 to 22). A 7z archive is assembled in memory and is limited to
1 GiB.

An empty input creates an empty archive.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `to` | `path` | required | Archive file to create. |
| `format` | `enum(auto\|zip\|tar\|tar_gz\|tar_zst\|tar_xz\|tar_bz2\|seven_zip)` | `auto` | Archive format; auto selects it from the file extension. |
| `base` | `path` | none | Directory that entry names in the archive are relative to. |
| `level` | `int` | none | Compression level of the format. |
| `follow_links` | `bool` | `false` | Store the targets of symbolic links instead of the links. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `entry` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

## Examples

Archive the build output of an application with names starting at `dist`:

```nurt example
find "/srv/app/dist" |> archive.create "/srv/out/dist.zip" base: "/srv/app"
```

```text expected
effect archive.create  /srv/out/dist.zip  files: 2  directories: 1
```

Archive the entries of a directory as a gzip-compressed tar archive:

```nurt example
list "/srv/app" |> archive.create "/srv/out/app.tar.gz"
```

Archive the sources as a zstd-compressed tar archive at a high level:

```nurt example
find "/srv/app/src" depth: 0 |> archive.create "/srv/out/app.bin" format: tar_zst level: 19
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_EXISTS` | `to` exists | Remove it first or choose another name. |
| `E_NOT_FOUND` | the directory of `to` or an input entry does not exist, or a followed link is dangling | Check the paths. |
| `E_INVALID_VALUE` | the format cannot be told from the extension of `to`, `level` is outside the range of the format, an entry lies outside `base`, or a name is not valid Unicode | Set `format`, correct `level` or `base`, or rename the entry. |
| `E_UNKNOWN_VALUE` | the value of `format` is unknown | Use one of the listed values. |
| `E_STALE` | an input file changed while it was stored, or `to` was created after the plan | Run the command again. |
| `E_LIMIT` | a 7z archive exceeds the in-memory limit | Create a zip or tar archive instead. |
| `E_TYPE` | an input record has no `path` field | Pass records of files. |

## Related

`archive.list` lists the members of the new archive; `archive.extract`
extracts it; `compress` compresses single files.
