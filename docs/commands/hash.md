---
command: hash
group: text
summary: Compute digests of file contents.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# hash

## Synopsis

```text
hash path... [algorithm: sha256|sha512|sha1|md5|blake3|crc32]
<entry records> |> hash [algorithm: sha256|sha512|sha1|md5|blake3|crc32]
```

## Description

`hash` computes a digest of the content of each file and returns it in the
`hash` field of an `entry` record, in the form `<algorithm>:<hex>`, for
example `sha256:ba7816bf...`. The hexadecimal part is the same as the output
of `sha256sum`, `sha512sum`, `sha1sum`, `md5sum` and `b3sum`; a CRC-32 is
written as eight hexadecimal digits in big-endian byte order, as by
`cksum -a crc32b` and `7z h`.

In the source form, `hash` returns one `entry` record per path, in the order
of the paths. A link is hashed by the content of its target, and its record
describes the link itself. A directory or another entry that is not a
regular file fails with `E_INVALID_VALUE`.

In the stage form, `hash` sets the `hash` field of every input `entry`
record and keeps its other fields. Records of directories and other entries
that are not regular files are passed on unchanged, without a `hash` field.
An empty input gives an empty output.

Content is read in chunks, so a file of any size is hashed in fixed memory.
With more than one thread in the limits of the call, files are hashed in
parallel; the output keeps the order of the input.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `list<path>` | | Files to hash in the source form; position 1, repeated. Relative paths are resolved against the base directory. Not allowed in the stage form. |
| `algorithm` | `enum(sha256\|sha512\|sha1\|md5\|blake3\|crc32)` | `sha256` | Digest algorithm. `sha1`, `md5` and `crc32` serve compatibility with existing checksums and do not protect against deliberate changes. |

## Input and output

| Form | Input | Output |
|---|---|---|
| source | none | `entry` with `hash` |
| stage | `entry` | `entry` with `hash` |

## Effect

Category `source`, effect `read`, reversibility `read`. The command reads
files only and starts no external program.

## Examples

Hash one file with SHA-256.

```nurt example
hash "/srv/app/release.txt"
```

```text expected
path                    hash
/srv/app/release.txt    sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
```

Hash two files with MD5.

```nurt example
hash "/srv/app/a.txt" "/srv/app/b.txt" algorithm: "md5"
```

```text expected
path              hash
/srv/app/a.txt    md5:900150983cd24fb0d6963f7d28e17f72
/srv/app/b.txt    md5:f96b697d7cb7938d525a2f31aaf161d0
```

Hash the files of a listing with BLAKE3 and group files with equal content.

```nurt example
find "/srv/app" type: file |> hash algorithm: "blake3" |> group by: hash
```

Compute the CRC-32 of a file.

```nurt example
hash "/srv/app/check.txt" algorithm: "crc32"
```

```text expected
path                  hash
/srv/app/check.txt    crc32:cbf43926
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | The source form has no `path`. | Give the files to hash or pass `entry` records. |
| `E_PARAM_CONFLICT` | `path` is given in the stage form. | Remove `path`, or start the pipeline with `hash`. |
| `E_UNKNOWN_VALUE` | `algorithm` names no supported algorithm. | Use one of `sha256`, `sha512`, `sha1`, `md5`, `blake3`, `crc32`. |
| `E_INVALID_VALUE` | A path of the source form is a directory or not a regular file (`problem: not_a_file`). | Hash the files of a directory with `find ... type: file \|> hash`. |
| `E_NOT_FOUND` | A file does not exist. | Check the path. |
| `E_PERMISSION` | A file cannot be read. | Check the permissions of the file. |
| `E_IO` | Reading a file failed. | Check the device and repeat the call. |
| `E_CANCELLED`, `E_TIMEOUT` | The call was cancelled or reached its time limit while hashing. | Repeat the call with a longer `timeout`. |

## Related

`info ... hash: sha256` returns the hash of one file together with its other
details. `find`, `list` and `group` combine with `hash` to find duplicate
files.
