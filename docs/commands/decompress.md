---
command: decompress
group: archive
summary: Decompress individually compressed files.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# decompress

## Synopsis

```text
decompress <path>... [name: value]...
```

## Description

`decompress` writes the decompressed form of each file next to it. The
algorithm (gzip, zstd, xz, bzip2 or lz4) is taken from the content of the
file; the name of the output is the name without the compression extension.
The extensions `.gz`, `.zst`, `.xz`, `.bz2`, `.lz4` are removed, and `.tgz`,
`.tzst`, `.txz`, `.tbz2` become `.tar`. Extensions are compared without regard
to case. A file whose name has none of these extensions fails with
`E_INVALID_VALUE`, because the output name cannot be derived; rename the
file first.

Streams made of several members or frames are decompressed to the
concatenation of the members. Content is streamed into a staged file of the
transaction, so memory use does not depend on the size of the file. The
output of one file is limited to 1 TiB; a stream that would produce more
fails with `E_LIMIT` and nothing is written.

With `keep: false` the compressed files are removed in the same transaction,
after every output has been written. A failure leaves them in place.

An output that exists fails with `E_EXISTS` before anything is changed; the
command never overwrites a file. In the stage form, records of directories
and other entries that are not regular files are skipped.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `list<path>` | none | Files to decompress in the source form; position 1, repeated. Relative paths are resolved against the base directory. Not allowed in the stage form. |
| `keep` | `bool` | `true` | Keep the compressed files. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none`, `entry` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

Notable entries: `nothing_matched` for an empty input.

## Examples

Decompress a gzip file and keep it.

```nurt example
decompress "/srv/app/app.log.gz"
```

```text expected
op 51  decompress  ok
targets 1 file
  + /srv/app/app.log
```

Decompress a zstd file and remove the compressed file.

```nurt example
decompress "/srv/app/data.csv.zst" keep: false
```

```text expected
op 52  decompress  ok
targets 1 file
  + /srv/app/data.csv
  - /srv/app/data.csv.zst
```

Decompress a compressed tar file to a tar file, then list its entries.

```nurt example
decompress "/srv/app/backup.tgz"
```

```text expected
op 53  decompress  ok
targets 1 file
  + /srv/app/backup.tar
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | The source form has no `path`. | Give the files to decompress or pass `entry` records. |
| `E_PARAM_CONFLICT` | `path` is given in the stage form. | Remove `path`, or start the pipeline with `decompress`. |
| `E_INVALID_VALUE` | The name has no compression extension (`problem: unknown_extension`), or a path of the source form is not a regular file (`problem: not_a_file`). | Rename the file to end in a compression extension. |
| `E_FORMAT` | The content is not gzip, zstd, xz, bzip2 or lz4 data (`problem: not_compressed`), or the stream is damaged or ends early. | Check the file or restore it from its source. |
| `E_LIMIT` | The decompressed output exceeds the limit (`problem: decompressed_size`). | Check that the file is not a compression bomb; read part of it with `read ... decompress: auto`. |
| `E_EXISTS` | The output file exists, or two sources have the same output name (`problem: output_exists`). | Remove or rename the existing file. |
| `E_NOT_FOUND` | A file does not exist. | Check the path. |
| `E_STALE` | A source changed after the plan. | Repeat the call. |
| `E_PERMISSION`, `E_IO`, `E_NO_SPACE` | A file cannot be read or the output cannot be written. | Check the permissions and the free space. |
| `E_CANCELLED`, `E_TIMEOUT` | The call was cancelled or reached its time limit. | Repeat the call with a longer `timeout`. |

## Related

`compress` reverses the command. `archive.extract` unpacks archives.
`read ... decompress: auto` reads a compressed file without writing the
decompressed form.
