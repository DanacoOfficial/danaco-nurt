---
command: compress
group: archive
summary: Compress files individually with gzip, zstd, xz, bzip2 or lz4.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# compress

## Synopsis

```text
compress path... [algorithm: gzip|zstd|xz|bzip2|lz4] [keep: bool] [level: int]
<entry records> |> compress [algorithm: gzip|zstd|xz|bzip2|lz4] [keep: bool] [level: int]
```

## Description

`compress` writes the compressed form of each file next to it. The name of the
output is the name of the file with the extension of the algorithm appended:
`.gz`, `.zst`, `.xz`, `.bz2` or `.lz4`. A name that already has the extension
is extended again, so `data.tar` becomes `data.tar.gz`.

Each file is a separate stream; to pack several files into one archive use
`archive.create`. Content is streamed into a staged file of the transaction,
so memory use does not depend on the size of the file. zstd and lz4 streams carry a checksum of the content.

With `keep: false` the sources are removed in the same transaction, after
every output has been written. A failure leaves the sources in place.

An output that exists fails with `E_EXISTS` before anything is changed; the
command never overwrites a file. In the stage form, records of directories
and other entries that are not regular files are skipped. A path given twice
is compressed once.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `list<path>` | | Files to compress in the source form; position 1, repeated. Relative paths are resolved against the base directory. Not allowed in the stage form. |
| `algorithm` | `enum(gzip\|zstd\|xz\|bzip2\|lz4)` | `gzip` | Compression algorithm. |
| `keep` | `bool` | `true` | Keep the original files. |
| `level` | `int` | algorithm default | Compression level. Ranges: gzip 0..=9, zstd 1..=22, xz 0..=9, bzip2 1..=9, lz4 0..=0. |

## Input and output

| Form | Input | Output |
|---|---|---|
| source | none | `effect` |
| stage | `entry` | `effect` |

## Effect

Category `change`, effect `local`, reversibility `reversible`. Every output
and every removed source is a target of one journal operation, so `undo`
removes the outputs and restores the sources. A source changed after the plan
fails with `E_STALE` and nothing is written. A preview shows the same changes
as the execution.

Notable entries: `nothing_matched` for an empty input.

## Examples

Compress a log file with gzip and keep it.

```nurt example
compress "/srv/app/app.log"
```

```text expected
op 41  compress  ok
targets 1 file
  + /srv/app/app.log.gz
```

Compress two files with zstd at level 19 and remove the originals.

```nurt example
compress "/srv/app/a.csv" "/srv/app/b.csv" algorithm: "zstd" level: 19 keep: false
```

```text expected
op 42  compress  ok
targets 2 files
  + /srv/app/a.csv.zst
  + /srv/app/b.csv.zst
  - /srv/app/a.csv
  - /srv/app/b.csv
```

Compress the old logs of a directory with xz.

```nurt example
find "/srv/app/logs" glob: "*.log" type: file |> compress algorithm: "xz"
```

```text expected
op 43  compress  ok
targets 1 file
  + /srv/app/logs/2026-01.log.xz
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | The source form has no `path`. | Give the files to compress or pass `entry` records. |
| `E_PARAM_CONFLICT` | `path` is given in the stage form. | Remove `path`, or start the pipeline with `compress`. |
| `E_UNKNOWN_VALUE` | `algorithm` names no supported algorithm. | Use one of `gzip`, `zstd`, `xz`, `bzip2`, `lz4`. |
| `E_INVALID_VALUE` | `level` is outside the range of the algorithm (`problem: level_out_of_range`), or a path of the source form is not a regular file (`problem: not_a_file`). | Use a level of the range in the table; compress the files of a directory with `find ... type: file \|> compress`. |
| `E_EXISTS` | The output file exists (`problem: output_exists`). | Remove or rename the existing file. |
| `E_NOT_FOUND` | A file does not exist. | Check the path. |
| `E_STALE` | A source changed after the plan. | Repeat the call. |
| `E_PERMISSION`, `E_IO`, `E_NO_SPACE` | A file cannot be read or the output cannot be written. | Check the permissions and the free space. |
| `E_CANCELLED`, `E_TIMEOUT` | The call was cancelled or reached its time limit. | Repeat the call with a longer `timeout`. |

## Related

`decompress` reverses the command. `archive.create` packs several files into
one archive. `read ... decompress: auto` reads a compressed file without
writing the decompressed form.
