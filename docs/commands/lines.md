---
command: lines
group: core
summary: Produce the lines of a file or of the input content.
since: 0.1.0
---

# lines

## Synopsis

```text
lines <path> [<range>]
<entry | content records> |> lines [<range>]
```

## Description

`lines` produces one `line` record per line of a text file. The source form
reads the given file; the encoding is detected, so UTF-16 and legacy
encodings such as Windows-1250 are decoded. The stage form reads the files of
input `entry` records and the text of input `content` records; directory
entries are passed over, and binary files are skipped and counted (notable
`binary_skipped`).

A line ends at a line feed, which may be preceded by a carriage return; a
text with carriage returns and no line feed is split at the carriage
returns. A last line without a line ending is a line, and a final line
ending does not start another line, so an empty file has no lines.

`range` selects lines by number from 1; lines past the end of the file are
not produced. Lines of a `content` record keep the numbers of the file, so
`read … lines: 10..20 |> lines` numbers them from 10.

`start` and `end` are byte offsets of the line in the file as stored,
without the line ending, also for files that are not UTF-8. They are left
out for `content` records that hold a range of lines.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required in the source form | File to read in the source form. |
| `range` | `range` | `all` | Line range, such as `10..20`, `40..`, `..9` or a single line number. |

## Input and output

| Form | Input | Output |
|---|---|---|
| source | none | `line` |
| stage | `entry`, `content` | `line` |

`line` fields: `path`, `no`, `text` (without the line ending),
`line_ending` (`lf`, `crlf`, `cr` or `none`), `etag`, `start`, `end`,
`encoding`.

## Effect

Category `source`, effect `read`, reversibility `read`. The command reads
files and changes nothing. It starts no external programs.

## Examples

Show the second and third line of a file.

```nurt example
lines "notes.txt" 2..3
```

```text expected
notes.txt:2: beta
notes.txt:3: gamma
```

Split the lines read by `read`, keeping the line numbers of the file.

```nurt example
read "notes.txt" lines: 3..4 |> lines
```

```text expected
notes.txt:3: gamma
notes.txt:4: delta
```

Show the first line of every text file found.

```nurt example
find "." glob: "*.txt" |> lines 1
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | No file in the source form. | Give `path`. |
| `E_INVALID_VALUE` | A bound of `range` is below 1. | Number lines from 1, as in `1..20`. |
| `E_TYPE` | `range` is not a range, or an input record is neither `entry` nor `content`. | Give a range such as `10..20`, or pass records of files. |
| `E_BINARY` | The file of the source form has binary content. | Use `read view: hex` for binary files. |
| `E_ENCODING` | The content is not valid in the detected encoding. | Read the file with `read encoding:` and convert it with `convert`. |
| `E_LIMIT` | The file is larger than the text limit. | Read a range of lines with `read lines:`. |
| `E_NOT_FOUND` | The file does not exist. | Check the path. |
| `E_PATH_RELATIVE` | A relative path is given without a base directory. | Give an absolute path or set the `base` option. |

## Related

`read`, `search`, `replace`, `insert`.
