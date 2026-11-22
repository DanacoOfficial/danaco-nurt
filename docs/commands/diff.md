---
command: diff
group: text
summary: Compare two files, or the input with a file or named result, and return the differences as hunks.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# diff

## Synopsis

```text
diff from to [context: int] [ignore_whitespace: bool]
<entry or content records> |> diff against [context: int] [ignore_whitespace: bool]
```

## Description

`diff` compares texts line by line and returns one `hunk` record per region
of change. For the same texts and number of context lines the hunks equal
those of GNU `diff -u`.

In the source form `from` is the old side and `to` the new side. Two files
give the hunks of their difference. Two directories are compared
recursively by relative path, in byte order of the paths: a file present
only in `to` is `added`, a file present only in `from` is `deleted`, and a
deleted file and an added file with the same non-empty content are reported
once as `renamed`. Links to regular files are compared as files; links to
directories are not followed. A file compared with a directory is compared
with the file of the same name in that directory.

In the stage form each input record is the new side and `against` is the old
side. An `entry` record names a file; a `content` record carries the text
itself, so a text read or changed earlier in the pipeline can be compared
without writing it. `against` is a file, a directory (each input file is
compared with the file of the same name in it, and an input directory with
the whole tree), or a named result whose records are matched to the input by
`path`; a named result of one record is compared with every input. An input
without a counterpart is `added`. Entries that are neither files nor
directories are skipped.

Files are decoded before the comparison, so files in different encodings are
compared by their characters, and the difference of encodings is reported as
`encoding_changed`. A carriage return belongs to the line, so lines with
`crlf` and `lf` endings differ unless `ignore_whitespace` is set. Files with
binary content are compared by content hash; a difference gives one record
with `binary: true` and no lines.

Equal inputs give no records.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `from` | `path` | | Old file or directory in the source form; position 1, required. |
| `to` | `path` | | New file or directory in the source form; position 2, required. |
| `against` | `path \| stream<any>` | | File, directory or named result that the input is compared with in the stage form; position 1, required. |
| `context` | `int` | `3` | Lines of context around each change; changes separated by at most twice this many lines share a hunk. Must be zero or more. |
| `ignore_whitespace` | `bool` | `false` | Treat lines that differ only in spaces, tabs, vertical tabs, form feeds and carriage returns as equal, as `diff -w`. Context lines are then taken from the old side. |

## Input and output

| Form | Input | Output |
|---|---|---|
| source | none | `hunk` |
| stage | `entry`, `content` | `hunk` |

Fields of the `hunk` record:

| Field | Value |
|---|---|
| `path` | File after the change; for a deleted file, the deleted file. |
| `old_path` | File before the change, when it differs from `path`. |
| `old_start`, `old_len`, `new_start`, `new_len` | Ranges of the hunk, as in `@@ -old_start,old_len +new_start,new_len @@`. |
| `lines` | Lines with the prefixes ` `, `-` and `+`. A line without a final line feed has no marker line. |
| `header` | Section heading after `@@`, when present. |
| `status` | `modified`, `added`, `deleted`, `renamed` or `binary`. |
| `binary` | `true` for files with binary content. |
| `id` | `path#number`, the number of the hunk within its file, from 1. |
| `fingerprint` | First six hexadecimal digits of the SHA-256 hash of `lines`, each followed by a line feed. |
| `ignore_whitespace` | `true` when white space was ignored. |

An added or deleted empty file and a renamed file give one record without
lines.

## Effect

Category `source`, effect `read`, reversibility `read`. The command reads
files and does not change them. Notable entries: `encoding_changed` with the
encodings of the old and new side in `detail`, and `portability` for a
directory entry whose name is not valid UTF-8, which is skipped.

## Examples

Compare two versions of a configuration file.

```nurt example
diff "/srv/app/config.old.toml" "/srv/app/config.toml"
```

```text expected
--- /srv/app/config.old.toml
+++ /srv/app/config.toml
@@ -1,3 +1,3 @@
 [server]
-timeout = 30
+timeout = 60
 port = 8080
```

Compare two release trees; the moved file is reported once as renamed.

```nurt example
diff "/srv/release/1.0" "/srv/release/1.1"
```

```text expected
added     /srv/release/1.1/CHANGELOG.md
deleted   /srv/release/1.0/NOTES.txt
modified  /srv/release/1.1/src/main.rs
renamed   /srv/release/1.0/src/util.rs -> /srv/release/1.1/src/run.rs
```

Compare the listed sources with the files of the same name in a copy, with
one line of context.

```nurt example
find "/srv/app/src" glob: "*.rs" type: file |> diff against: "/srv/app.orig/src" context: 1
```

Compare two scripts without regard to indentation and line endings.

```nurt example
diff "/srv/app/build.cmd" "/srv/app/build.new.cmd" ignore_whitespace: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `from` or `to` is missing in the source form, or `against` in the stage form. | Give both sides. |
| `E_PARAM_CONFLICT` | `against` is given in the source form, or `from` or `to` in the stage form. | Use the parameters of the form. |
| `E_INVALID_VALUE` | `context` is negative, or a path is neither a regular file nor a directory (`problem: not_a_file`). | Give zero or more lines, and files or directories. |
| `E_TYPE` | `against` is neither a path nor a named result of `entry` or `content` records, or a `content` record has no `text`. | Pass a path or the result of `read`. |
| `E_LIMIT` | A file is larger than the limit of text files. | Compare smaller files. |
| `E_ENCODING` | A file is not valid in its detected encoding. | Convert the file with `convert`. |
| `E_NOT_FOUND`, `E_PERMISSION`, `E_IO` | A file cannot be read. | Check the path and its permissions. |

## Related

`patch` applies a unified or git diff. `read ... since:` returns the
difference of a file against the version read earlier. `hash` compares files
by content without reading their lines.
