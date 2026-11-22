---
command: archive.list
group: archive
summary: List the entries of a zip, tar, compressed tar, 7z, jar, whl or apk archive.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# archive.list

## Synopsis

```text
archive.list path [glob: glob]
```

## Description

`archive.list` produces one `archive_entry` record for every member of the
archive `path`, in the order of the archive. The format is recognised from
the content of the file, not from its name: zip (including jar, whl and apk),
tar, tar compressed with gzip, zstd, xz, bzip2 or lz4, and 7z.

Every record carries the archive, the member path with `/` separators, the
name, the type (`file`, `directory`, `link` or `other`), the size after
extraction, the size in the archive when the format records it per member,
the modification time, the permissions in octal, the target of a link, the
compression method, whether the content is encrypted, the CRC-32 checksum
when the format records it, and the format of the archive. Paths and names
are in NFC; a name stored with bytes that are not valid UTF-8 is shown with
U+FFFD in their place.

`glob` selects members by path. A pattern without `/` is matched against the
last component of the member path, a pattern with `/` against the whole
path; `*` and `?` do not match `/`, `**` matches any number of components.

A compressed tar archive and a solid 7z archive are decoded from the start,
so their members are collected before the first record is produced. An empty
archive produces no records.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Archive file; position 1. |
| `glob` | `glob` | none | Only members whose path matches this pattern. |

## Input and output

Input: none; `archive.list` starts a pipeline.

Output: `archive_entry` records.

## Effect

Category `source`, effect `read`, reversibility `read`; no external
programs. `archive.list` changes nothing and writes no journal entry.

## Examples

List the members of a zip archive:

```nurt example
archive.list "/home/user/dl/site.zip"
```

```text expected
css           directory
css/site.css  file  4 B
index.html    file  13 B
```

List only the Rust sources of a compressed tar archive:

```nurt example
archive.list "/home/user/dl/src.tar.gz" glob: "*.rs"
```

List a zstd-compressed tar archive whose name has no archive extension; the
`format` field of the records is `tar_zst`:

```nurt example
archive.list "/home/user/dl/backup.bin"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path. |
| `E_FORMAT` | the file is not an archive of a supported format, or the archive is damaged | Check the file; use `decompress` for a compressed file that holds no tar archive. |
| `E_ENCRYPTED` | the header of a 7z archive is encrypted | Decrypt the archive with its own tool first. |
| `E_INVALID_VALUE` | `glob` is not a valid pattern | Correct the pattern. |
| `E_LIMIT` | the archive has more members than the record limit | Select fewer members with `glob`. |
| `E_PATH_RELATIVE` | `path` is relative and no base directory is set | Give an absolute path or set the `base` option. |

## Related

`archive.extract` extracts an archive; `archive.create` creates one;
`decompress` decompresses a single compressed file.
