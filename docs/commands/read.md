---
command: read
group: core
summary: Read text, images, PDF pages, notebooks or document keys.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# read

## Synopsis

```text
read <path>... (path: <list<path>> | targets: <list<map<any>>>) [name: value]...
entry |> read [name: value]...
```

## Description

The command reads files and produces one `content` record per file, or
`item` records when `key` is given. In the source form the files are the
paths given, in order, or the maps of `targets`; each map names a `path` and
at most one selector of its own. In the stage form the files are those of the
input `entry` records; directories and special entries among them are
skipped.

A file is decoded in the encoding detected for it, or in the encoding given
with `encoding`. Content that is not text in any supported encoding fails
with `E_BINARY` unless `view: hex` is given. The `etag` field of the record
carries the SHA-256 hash of the file bytes and can be passed to `if_match` of
the changing commands.

Lines count from 1 and end at a line feed; a carriage return before the line
feed belongs to the line ending. `search` counts lines the same way. The text
of the record keeps the original line endings.

A call takes at most one selector:

| Selector | Result |
|---|---|
| none | the whole text |
| `lines` | the lines of one or more ranges, in the order given; overlapping ranges are merged into the place of the first of them; the end of a range beyond the last line is reduced to the last line, and a range that starts after it fails with `E_OUT_OF_RANGE` |
| `last` | the last lines; the whole text when the file is shorter |
| `around` | the lines of every match of the text or regular expression with `context` lines before and after; spans that overlap or touch are merged |
| `section` | a Markdown section: its ATX or Setext heading and the lines up to the next heading of the same or a higher level; nested headings are joined with ` > `; headings in fenced code are ignored, and emphasis, code and link markers are ignored when headings are compared |
| `symbol` | the definition of a code symbol; a qualified name such as `Server.start` narrows the container |
| `key` | `item` records of the values at a key path of a structured document, expanded to `depth` levels |
| `cell` | one cell of a notebook, by index from 0 or by identifier |

The record has `range` for one span of lines and `ranges` for none or
several. `total_lines` is the number of lines of the file and `lines` the
number of lines in `text`.

Files with the extension `.ipynb` are read as notebooks: the record has the
cells in `cells` and their sources in `text`.

`view` selects what the record holds: `full` the text; `header` the metadata
without text; `outline` the symbols of the file in `symbols` and one indented
line per symbol in `text`; `symbols` the symbols without text;
`outline_first` the symbols and the text of the selection. `header`,
`outline` and `symbols` take no selector. Without code analysis, or when no
grammar recognises the file, the views, `symbol` and `key` return the whole
text and report the notable effect `semantic_fallback`.

Conditional reads avoid sending text that is already known. Each of
`unchanged_since`, `since` and the values of `known` is an etag, a content
hash `sha256:…` or at least 8 leading hexadecimal digits of one. With
`unchanged_since` a file that still has that version gives a record with
`unchanged: true` and no text. With `since` the record holds the hunks of the
difference against that version in `diff`, without text, when the version is
in the read cache; otherwise it holds the text and `since_missing: true`.
Every text read stores the text in the cache, which keeps 64 MiB for one
hour. Files named in `known` that still have the given version are left out
of the result.

`max_line` cuts longer lines after that many characters and appends
`[N characters omitted]`; the record then has `truncated: true` and the
notable effect `truncated_lines` gives the number of lines cut.

### PDF documents

A file that starts with `%PDF-` is a PDF document, also when it consists of
ASCII characters only; a named `encoding` reads its source as text. A PDF
document gives one `content` record per page, with the page number in
`page` and the number of pages of the document in `pages_total`. `pages`
selects pages as one or more ranges, read in the order given; the end of a
range beyond the last page is reduced to the last page, and a range that
starts beyond it fails with `E_OUT_OF_RANGE`. `mode: text` extracts words and
lines in reading order; `mode: layout` places every word at its horizontal
position on a character grid, which keeps columns and tables aligned.
`mode: images` requires the PDFium library and fails with
`E_EXTERNAL_MISSING` in builds without it. `view: header` gives one record
with `pages_total` and no text. A document that requires a password fails
with `E_ENCRYPTED`; a document encrypted only to restrict printing or copying
is read. Text comes from the fonts of the document, so pages that carry text
only as images give empty text. Selectors of lines do not apply to documents
and fail with `E_UNSUPPORTED`.

### Images

An image gives one record with `binary: true`, empty `text` and `image`: the
media type in `mime`, the width and height in pixels, read from the image
header without decoding the pixels, and the bytes of the file in `data`.
`view: header` reads only the header and leaves out `data`. Types recognised
without a decoder, such as HEIF, have no dimensions.

### Hexadecimal dump

`view: hex` dumps the bytes of any file in the layout of `xxd`: the offset of
the row as eight or more hexadecimal digits, a colon, 16 bytes in groups of
two and the same bytes as ASCII, with `.` for bytes outside the printable
range. Each row is one line of `text`; `lines`, `last` and `pages` select
rows, counted from 1, and `total_lines` is the number of rows. Rows of a file
are read by position, so a range of a large file reads only that range. The
text of one dump is limited to the text limit and fails with `E_LIMIT` above
it.

### Compressed files

With `decompress: auto` a file whose name ends with `.gz`, `.zst`, `.xz` or
`.bz2`, or with `.tgz`, `.tzst`, `.txz` or `.tbz2`, and whose content starts
with the signature of the same format is decompressed while it is read. The
decompressed content is read as text, as a PDF document or as an image, and
`view: hex` dumps the decompressed bytes. The records keep the `path` and
the `etag` of the compressed file and name it in `decompressed_from`; the
notable effect `decompressed` is reported. Decompressed content larger than
the text limit fails with `E_LIMIT`, so a small file that expands without
bound is stopped. `decompress: none` reads the compressed bytes. With
`since`, the earlier text of a compressed file is found only in the read
cache shared by the session.

Records of PDF documents, images and dumps have `encoding: binary`,
`bom: false` and `line_ending: none`. Their `etag` carries the content hash
when the whole file was read, which is not the case for dumps and image
headers. `unchanged_since` and `known` apply to them; `since` returns the
content with `since_missing: true`.

### Encoding detection

With `encoding: auto` the signatures of binary formats are checked first: a
file that starts with `%PDF-` is a PDF document, a file read with
`decompress: auto` whose name and signature agree on a compression format is
decompressed, and a file that starts with the signature of an image is an
image unless its leading bytes are plain ASCII text. The first 64 KiB of any
other file decide its encoding, in this order: a byte order mark selects UTF-8, UTF-16 LE or UTF-16 BE; valid UTF-8
without NUL characters is UTF-8; content in which one code unit in ten or
more has a zero high byte is UTF-16 without a byte order mark; a NUL byte
makes the content binary; otherwise a statistical detector proposes a legacy
encoding, and the encodings of the configuration key `text.legacy_encodings`
(`windows-1250`, `ibm852`, `iso-8859-2` and `windows-1252` by default)
replace the proposal when one of them reads the content as Polish text.
Content with more than one control character in twenty characters is binary.

- The legacy encodings detected are `windows-1250`, `ibm852`, `iso-8859-2`
  and `windows-1252`. Content in another code page, such as `windows-1251`,
  is read in the most plausible of them.
- Samples shorter than 200 bytes contain few letters outside ASCII, so the
  choice between legacy encodings is less certain for them; give `encoding`
  for such files.
- UTF-16 without a byte order mark is recognised only for text in which at
  least one character in ten lies in the range U+0000 to U+00FF, such as
  text in Latin script; other UTF-16 text without a byte order mark is
  binary.
- A PDF document that consists of ASCII characters only is read as a PDF
  document; a named `encoding` reads its source as text.
- Image signatures that consist of ASCII letters, such as `BM` and `GIF`,
  do not apply to plain ASCII text, so a text file that starts with them is
  read as text.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `list<path>` | one of `target` | Files to read. |
| `lines` | `range \| list<range>` | none | Line ranges to read. |
| `last` | `int` | none | Number of last lines to read. |
| `around` | `text \| regex` | none | Read the lines around matches of this pattern. |
| `context` | `int` | `3` | Lines of context around matches of around. |
| `symbol` | `text` | none | Read the definition of this code symbol. |
| `section` | `text` | none | Read the Markdown section with this heading. |
| `cell` | `int \| text` | none | Notebook cell by index or identifier. |
| `targets` | `list<map<any>>` | one of `target` | Selectors per target: path, lines, around, symbol, key, section, cell. |
| `key` | `key` | none | Key path in a structured document; the result is item records. |
| `depth` | `int` | none | Depth of nested values read at key. |
| `view` | `enum(full\|outline\|outline_first\|symbols\|header\|hex)` | `full` | Full content, outline, symbols, header or hexadecimal dump. |
| `pages` | `range \| list<range>` | none | PDF pages to read. |
| `mode` | `enum(text\|layout\|images)` | `text` | Extraction mode of PDF pages. |
| `encoding` | `text` | `"auto"` | Text encoding; auto detects it. |
| `decompress` | `enum(auto\|none)` | `auto` | Read compressed files after decompressing them on the fly. |
| `unchanged_since` | `text` | none | Etag; an unchanged file is returned without text. |
| `since` | `text` | none | Etag of an earlier read; the result holds the differences since then. |
| `known` | `map<text>` | none | Map from path to etag; unchanged files are skipped. |
| `line_numbers` | `bool` | `false` | Prefix lines with their numbers. |
| `max_line` | `int` | no limit | Maximum length of a returned line in characters; longer lines are cut and marked. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `source` | `none` | `content`, `item` |
| `stage` | `entry` | `content`, `item` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Read the lines around the start of `main` with three lines of context and
line numbers.

```nurt example
read "/srv/app/main.go" around: /func main/ context: 3 line_numbers: true
```

```text expected
41  func helper() {}
42  
43  // main starts the server.
44  func main() {
45  	helper()
46  }
47  
```

Read the file again only when it changed since the version with the content
hash starting with `1a2b3c4d`.

```nurt example
read "/srv/app/main.go" unchanged_since: "1a2b3c4d"
```

```text expected
unchanged: true
```

Read several line ranges; the overlapping ranges `5..8` and `7..10` are
merged in the place of the first.

```nurt example
read "/srv/app/CHANGELOG.md" lines: [5..8, 1..2, 7..10]
```

```text expected
ranges: [5..10, 1..2]
```

Read the subsection `Linux` of the section `Install` of a Markdown file.

```nurt example
read "/srv/app/README.md" section: "Install > Linux"
```

```text expected
### Linux

apt install app

```

Read the last two lines of a log.

```nurt example
read "/var/log/app.log" last: 2
```

```text expected
request 2
stop
```

Read pages 2 and 3 of a PDF document.

```nurt example
read "/srv/docs/report.pdf" pages: 2..3
```

```text expected
page: 2, pages_total: 3
Revenue grew in the third quarter.
page: 3, pages_total: 3
Costs fell by four percent.
```

Dump the first two rows of the bytes of an executable.

```nurt example
read "/srv/app/bin/app" view: hex lines: 1..2
```

```text expected
00000000: 7f45 4c46 0201 0100 0000 0000 0000 0000  .ELF............
00000010: 0300 3e00 0100 0000 4010 0000 0000 0000  ..>.....@.......
```

Read the last line of a compressed log.

```nurt example
read "/var/log/app.log.gz" last: 1
```

```text expected
stop
decompressed_from: /var/log/app.log.gz
```

Read the type and dimensions of an image without its data.

```nurt example
read "/srv/site/logo.png" view: header
```

```text expected
image: {mime: image/png, width: 64, height: 32}
```

A response that leaves lines out ends with the hint "more: next" and carries the field
`continuation`; the command `next` returns the next part without running the
command again.

```text
shown 50  more: next
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | The file does not exist. | Check the path, for example with `find`. |
| `E_PERMISSION` | The file cannot be read. | Read a file the account may read. |
| `E_INVALID_VALUE` | A directory or special file was given, a line number is below 1, `last` or `max_line` is below 1, a value is not an etag, or `cell` was given for a file that is not a notebook. | Give a file, or a valid value. |
| `E_OUT_OF_RANGE` | A range of `lines` starts after the last line of the file, which includes every range of an empty file, or the index of `cell` is not below the number of cells. `found` holds the range and `actual` the number of lines or cells. | Use lines from 1 to `total_lines`, or a cell index below `actual`. |
| `E_PARAM_CONFLICT` | More than one selector, selectors both for the call and in `targets`, or a selector with `view: header`, `outline` or `symbols`. | Use one selector, or `view: outline_first`. |
| `E_MISSING_PARAM` | Neither `path` nor `targets`, or a target without `path`. | Give the files to read. |
| `E_UNKNOWN_FIELD` | A map of `targets` has an unknown key. | Use the keys listed in `expected`. |
| `E_BINARY` | The content is not text. | Read it with `view: hex`. |
| `E_ENCODING` | The content is not valid in the encoding given with `encoding`. | Use `encoding: auto` or the encoding of the file. |
| `E_LIMIT` | The text is larger than the text limit. | Search the file with `search`. |
| `E_REGEX` | The pattern of `around` is not a valid regular expression. | Correct the pattern. |
| `E_KEY_NOT_FOUND` | The section or key does not exist, or no cell has the identifier given with `cell`. | Use a heading listed in `expected`, or an existing key or cell identifier. |
| `E_SYMBOL_NOT_FOUND` | The file defines no such symbol. | List the symbols with `view: symbols`. |
| `E_SYMBOL_AMBIGUOUS` | The name denotes several symbols. | Qualify the name with its container. |
| `E_UNSUPPORTED_LANGUAGE` | No grammar recognises the file given with `symbol`. | Read lines or a section instead. |
| `E_UNSUPPORTED` | The file given with `key` is not a structured document of a known format. | Read lines instead. |
| `E_FORMAT` | A notebook is not valid JSON of the notebook format. | Read its text with `lines`, which does not parse the notebook. |

`last` larger than the number of lines gives the whole text, and `around`
without a match gives empty text; neither is an error.

### Errors of PDF documents, images, dumps and compressed files

| Code | Meaning | Typical fix |
|---|---|---|
| `E_OUT_OF_RANGE` | A range of `pages` starts after the last page of the document. | Use pages from 1 to `pages_total`. |
| `E_INVALID_VALUE` | A page or row number of `pages` is below 1. | Count pages and rows from 1. |
| `E_ENCRYPTED` | The PDF document requires a password. | Read a copy without a password. |
| `E_FORMAT` | The PDF document, the image header or the compressed data is damaged. | Check that the file is complete, or read it with `decompress: none` or `view: hex`. |
| `E_EXTERNAL_MISSING` | `mode: images` was given in a build without the PDFium library. | Use `mode: text` or `mode: layout`. |
| `E_UNSUPPORTED` | A selector of lines or a view of text was given for a PDF document or an image, or a selector other than `lines` and `last` with `view: hex`. | Select pages with `pages`, or rows with `lines`, `last` or `pages`. |
| `E_PARAM_CONFLICT` | `view: hex` with more than one of `lines`, `last` and `pages`. | Select the rows once. |
| `E_LIMIT` | The document, the image, the decompressed content or the dump is larger than the text limit. | Dump selected rows with `view: hex` and `lines`, or read with `decompress: none`. |

## Related

`lines` produces one `line` record per line, `search` finds matches in many
files, `info` reports metadata, `notebook.cells` lists notebook cells and
`diff` compares two files.
