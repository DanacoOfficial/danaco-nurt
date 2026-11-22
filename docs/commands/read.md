---
command: read
group: core
summary: Read text, images, PDF pages, notebooks or document keys.
since: 0.1.0
---

# read

## Synopsis

```text
read <path>… [lines: <range> | <list<range>>] [last: <int>] [around: <text> | <regex>] [context: <int>] [symbol: <text>] [section: <text>] [cell: <int> | <text>] [key: <key>] [depth: <int>] [view: full|outline|outline_first|symbols|header|hex] [pages: <range> | <list<range>>] [mode: text|layout|images] [encoding: <text>] [decompress: auto|none] [unchanged_since: <text>] [since: <text>] [known: <map<text>>] [line_numbers: <bool>] [max_line: <int>]
read targets: <list<map<any>>> [view: …] [context: <int>] [depth: <int>] [encoding: <text>] [unchanged_since: <text>] [since: <text>] [known: <map<text>>] [line_numbers: <bool>] [max_line: <int>]
<entry records> |> read [lines: …] [last: …] [around: …] [context: …] [symbol: …] [section: …] [cell: …] [key: …] [depth: …] [view: …] [encoding: …] [unchanged_since: …] [since: …] [known: …] [line_numbers: …] [max_line: …]
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
| `lines` | the lines of one or more ranges, in the order given; overlapping ranges are merged into the place of the first of them; lines beyond the end of the file are left out |
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

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `list<path>` | — | Files to read; positional and repeated. Exclusive with `targets`. |
| `targets` | `list<map<any>>` | — | Selectors per target, with the keys `path`, `lines`, `around`, `symbol`, `key`, `section` and `cell`; `path` is required. |
| `lines` | `range \| list<range>` | — | Line ranges to read, such as `5`, `1..5`, `40..` or `..9`. |
| `last` | `int` | — | Number of last lines to read, at least 1. |
| `around` | `text \| regex` | — | Read the lines around matches of this pattern. |
| `context` | `int` | `3` | Lines of context around matches of `around`. |
| `symbol` | `text` | — | Read the definition of this code symbol. |
| `section` | `text` | — | Read the Markdown section with this heading; nested headings are joined with ` > `. |
| `cell` | `int \| text` | — | Notebook cell by index or identifier. |
| `key` | `key` | — | Key path in a structured document; the result is `item` records. |
| `depth` | `int` | — | Depth of nested values read at `key`, and of the symbols of the outline views. |
| `view` | `enum(full\|outline\|outline_first\|symbols\|header\|hex)` | `full` | Full content, outline, symbols, header or hexadecimal dump. |
| `pages` | `range \| list<range>` | — | PDF pages to read. |
| `mode` | `enum(text\|layout\|images)` | `text` | Extraction mode of PDF pages. |
| `encoding` | `text` | `"auto"` | Text encoding, such as `utf-8` or `windows-1250`; `auto` detects it. |
| `decompress` | `enum(auto\|none)` | `auto` | Read compressed files after decompressing them on the fly. |
| `unchanged_since` | `text` | — | Etag; an unchanged file is returned without text. |
| `since` | `text` | — | Etag of an earlier read; the result holds the differences since then. |
| `known` | `map<text>` | — | Map from path to etag; unchanged files are skipped. |
| `line_numbers` | `bool` | `false` | Prefix lines with their numbers, right-aligned and followed by two spaces. |
| `max_line` | `int` | unbounded | Maximum length of a returned line in characters; longer lines are cut and marked. |

## Input and output

| Form | Input | Output |
|---|---|---|
| source | none | `content`; `item` with `key` |
| stage | `entry` | `content`; `item` with `key` |

## Effect

Category `source`, effect `read`, reversibility `read`. The command changes
nothing and starts no external program.

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

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | The file does not exist. | Check the path, for example with `find`. |
| `E_PERMISSION` | The file cannot be read. | Read a file the account may read. |
| `E_INVALID_VALUE` | A directory or special file was given, a line number is below 1, `last` or `max_line` is below 1, a value is not an etag, or `cell` was given for a file that is not a notebook. | Give a file, or a valid value. |
| `E_PARAM_CONFLICT` | More than one selector, selectors both for the call and in `targets`, or a selector with `view: header`, `outline` or `symbols`. | Use one selector, or `view: outline_first`. |
| `E_MISSING_PARAM` | Neither `path` nor `targets`, or a target without `path`. | Give the files to read. |
| `E_UNKNOWN_FIELD` | A map of `targets` has an unknown key. | Use the keys listed in `expected`. |
| `E_BINARY` | The content is not text. | Read it with `view: hex`. |
| `E_ENCODING` | The content is not valid in the encoding given with `encoding`. | Use `encoding: auto` or the encoding of the file. |
| `E_LIMIT` | The text is larger than the text limit. | Search the file with `search`. |
| `E_REGEX` | The pattern of `around` is not a valid regular expression. | Correct the pattern. |
| `E_KEY_NOT_FOUND` | The section, key or cell does not exist. | Use a heading listed in `expected`, or an existing key or cell. |
| `E_SYMBOL_NOT_FOUND` | The file defines no such symbol. | List the symbols with `view: symbols`. |
| `E_SYMBOL_AMBIGUOUS` | The name denotes several symbols. | Qualify the name with its container. |
| `E_UNSUPPORTED_LANGUAGE` | No grammar recognises the file given with `symbol`. | Read lines or a section instead. |
| `E_UNSUPPORTED` | The file given with `key` is not a structured document of a known format. | Read lines instead. |
| `E_FORMAT` | A notebook is not valid JSON of the notebook format. | Read its text with `lines`, which does not parse the notebook. |

## Related

`lines` produces one `line` record per line, `search` finds matches in many
files, `info` reports metadata, `notebook.cells` lists notebook cells and
`diff` compares two files.
