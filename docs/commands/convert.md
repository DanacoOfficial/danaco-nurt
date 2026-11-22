---
command: convert
group: text
summary: Convert the text encoding, line endings or byte order mark of files.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# convert

## Synopsis

```text
convert <path>... [name: value]...
```

## Description

`convert` rewrites text files in another encoding, with other line endings
or with the byte order mark added or removed. At least one of `encoding`,
`line_ending` and `bom` must be given.

The encoding and the line endings of each file are detected as by `read`;
encodings other than UTF-8 and UTF-16 are recognised from the list of the
configuration key `text.legacy_encodings`. The whole conversion is checked
before any file changes: a character the target encoding cannot represent
fails with `E_ENCODING`, with the path, line and column of the character,
and no file is written. A file that already has the requested form is not
written and is reported as `unchanged`.

`line_ending` converts every line ending, also in files with mixed line
endings; a missing line ending at the end of the file stays missing. Without
`bom`, a file that keeps its encoding keeps its byte order mark, a file
converted to UTF-16 receives one, and a file converted to any other encoding
is written without one. A byte order mark exists only in UTF-8 and UTF-16.

In the source form every path must be a text file. In the stage form the
input `entry` records name the files; records of directories and of other
entries that are not regular files are skipped, and files with binary
content are skipped and counted in `binary_skipped`. A path given twice is
converted once.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `list<path>` | none | Files to convert in the source form. |
| `encoding` | `text` | none | Target encoding, such as utf-8 or windows-1250. |
| `line_ending` | `enum(lf\|crlf\|cr\|keep)` | `keep` | Target line ending, or keep to leave line endings unchanged. |
| `bom` | `bool` | none | Add or remove the byte order mark. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none`, `entry` | `effect` |

The `effect` record has one `change` per converted file with
`encoding_before`, `encoding_after`, `line_ending_before`,
`line_ending_after`, `bom_before`, `bom_after`, `size_before` and
`size_after`.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

Notable entries: `encoding_changed` and `line_ending_changed` with the values
before and after in `detail`, `bom_changed` with `added` or `removed`,
`unchanged` for files already in the requested form, `binary_skipped` and
`nothing_matched` for an empty input.

## Examples

Convert a Windows-1250 file with CRLF line endings to UTF-8 with LF.

```nurt example
convert "/srv/legacy/notes.txt" encoding: "utf-8" line_ending: lf
```

```text expected
op 31  convert  ok
targets 1 file
  ~ /srv/legacy/notes.txt  windows-1250 -> utf-8, crlf -> lf
notable
  i encoding_changed  /srv/legacy/notes.txt  windows-1250 -> utf-8
  i line_ending_changed  /srv/legacy/notes.txt  crlf -> lf
```

Convert the line endings of the listed scripts to CRLF.

```nurt example
find "/srv/app/scripts" glob: "*.cmd" type: file |> convert line_ending: crlf
```

```text expected
op 32  convert  ok
targets 1 file
  ~ /srv/app/scripts/build.cmd  lf -> crlf
notable
  i line_ending_changed  /srv/app/scripts/build.cmd  lf -> crlf
  i unchanged  /srv/app/scripts/test.cmd
```

Remove the byte order mark of a CSV file.

```nurt example
convert "/srv/app/data.csv" bom: false
```

```text expected
op 33  convert  ok
targets 1 file
  ~ /srv/app/data.csv  bom removed
notable
  i bom_changed  /srv/app/data.csv  removed
```

Convert a file of the DOS code page 852, with `ibm852` in
`text.legacy_encodings`, to UTF-8.

```nurt example
convert "/srv/dos/menu.txt" encoding: "utf-8"
```

A character that Latin-2 cannot represent stops the conversion.

```nurt example
convert "/srv/app/notes.txt" encoding: "iso-8859-2"
```

```text expected
error E_ENCODING  stage 4  /srv/app/notes.txt:2:8
  character `→` (U+2192) at line 2, column 8 cannot be represented in `iso-8859-2`
  fix: convert "/srv/app/notes.txt" encoding: "utf-8"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | No `path` in the source form, or none of `encoding`, `line_ending` and `bom`. | Give the files and the form to convert to. |
| `E_PARAM_CONFLICT` | `path` is given in the stage form. | Remove `path`, or start the pipeline with `convert`. |
| `E_UNKNOWN_VALUE` | `encoding` or `line_ending` has an unknown value. | Use a listed encoding or line ending. |
| `E_INVALID_VALUE` | `bom: true` for an encoding without a byte order mark (`problem: bom_not_supported`), or a path that is not a regular file (`problem: not_a_file`). | Remove `bom`, or convert the files of a directory with `find ... type: file \|> convert`. |
| `E_ENCODING` | A file is not valid in its detected encoding, or a character cannot be represented in the target encoding; `details` give `character`, `line`, `column` and `byte`. | Convert to `utf-8`, or remove the character. |
| `E_BINARY` | A path of the source form has binary content. | Give text files only. |
| `E_LIMIT` | A file is larger than the limit of text files. | Convert smaller files. |
| `E_STALE` | A file changed after the plan. | Repeat the call. |
| `E_NOT_FOUND` | A file cannot be read or written. | Check the path and its permissions. |
| `E_PERMISSION` | A file cannot be read or written. | Check the path and its permissions. |
| `E_IO` | A file cannot be read or written. | Check the path and its permissions. |

## Related

`read ... encoding:` reads a file in a given encoding. `write`, `replace`,
`insert` and `edit` keep the encoding and line endings of a file; `convert`
is the explicit way to change them. `undo` restores the previous content.
