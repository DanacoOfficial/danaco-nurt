---
command: replace
group: core
summary: Replace text in files or in the found text.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# replace

## Synopsis

```text
replace <pattern> <with> in: <list<path>> [name: value]...
match|node|item |> replace <with> [name: value]...
line|content |> replace <pattern> <with> [name: value]...
```

## Description

`replace` changes text in files. The source form replaces the matches of
`pattern` in the files listed in `in`; a directory of `in` contributes its
files that match `glob` and `file_type`. The form on `line` and `content`
records replaces the matches of `pattern` inside the lines of the records:
a `line` record selects its line, a `content` record its `range` or the
whole file. The form on `match` records replaces the text of every input
match, for example the output of `search`.

A text pattern is literal. It is matched with the tolerance of `whitespace`:
with `normalize`, the default, differences of indentation, trailing white
space, line endings and runs of white space are tolerated, an exact
occurrence always takes precedence, and the replacement takes the
indentation, the indentation unit and the line endings of the place it
replaces. A regular expression `/…/` is matched exactly. `^` and `$` match at
line boundaries, and a match does not span lines unless `multiline` is set,
which also lets `.` match a line break. A regular expression with
look-around or back-references is evaluated by the backtracking engine.

For regular expressions and input matches `with` is a template: `$1` and
`${1}` insert a numbered group, `${name}` a named group, `$0` the whole match
and `$$` a `$`. Any other `$` is an error. With `template: false` the text is
inserted as written. For input matches the numbered groups are the `groups`
of the record and the named groups its `named` map. A literal pattern always
inserts `with` as written.

`expect` counts the matches of all files before `occurrence` applies;
`occurrence` keeps one match, counted from 1, in every file. When the count
is outside `expect` nothing is changed and the error lists the first
matches. When nothing matches, `on_missing: skip` reports the notable effect
`nothing_matched` and changes nothing, and `on_missing: error` fails with
the nearest fragments of the text. When the pattern matches but no file has
the occurrence, `on_missing: error` fails with `E_OUT_OF_RANGE` and lists
the first matches.

`scope` keeps the matches that start in code, comments or string literals,
as classified by the grammar of the file. `mode: syntax` matches a
structural pattern written in the syntax of the language of the file, and
`with` is a structural template. Both need a grammar for the file.

With `check_references`, the files are searched after the change for the
replaced text outside the replacements: symbolically when a grammar knows
the file and the text is an identifier, otherwise as a whole word. The
places found are reported as the notable effect `references_left` with
their number and the first places; they do not prevent the change.

Every file keeps its encoding, byte order mark and line endings. A file
whose encoding cannot represent the new text is not changed. Binary files
found in a directory are skipped and counted as `binary_skipped`; a binary
file listed in `in` is skipped as `binary_target`. With `binary: text` a
binary file is read as Windows-1252 text.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `pattern` | `text \| regex` | required | Text or regular expression to replace. |
| `with` | `text` | required | Replacement; $1, ${name} and $$ refer to groups unless template is false. |
| `in` | `list<path>` | required | Files or directories in the source form. |
| `glob` | `glob \| list<glob>` | `"**"` | Name patterns of the files in directories. |
| `mode` | `enum(auto\|literal\|regex\|syntax)` | `auto` | Pattern mode; auto treats text as literal and /…/ as a regular expression. |
| `occurrence` | `int` | every occurrence | Replace only this occurrence, counted from 1, in every file. |
| `case_sensitive` | `bool` | `true` | Match with case sensitivity. |
| `whole_word` | `bool` | `false` | Match whole words only; a word consists of letters, digits and `_`. |
| `multiline` | `bool` | `false` | Allow matches of regular expressions across lines. |
| `whitespace` | `enum(exact\|trailing\|indent\|normalize)` | `normalize` | Tolerance of whitespace differences; normalize also adapts the replacement to the file style. |
| `scope` | `enum(any\|code\|comments\|strings)` | `any` | Part of source code in which matches are replaced. |
| `check_references` | `bool` | `false` | Report references to the replaced text left after the change. |
| `template` | `bool` | `true` | Treat the replacement as a template with group references. |
| `on_missing` | `enum(skip\|error)` | `skip` | Behaviour when nothing matches. |
| `expect` | `range` | none | Expected number of matches of all files; otherwise nothing is changed. |
| `if_match` | `text \| map<text>` | none | Expected etag of the file, or a map from path to etag for several files. |
| `validate` | `enum(off\|warn\|reject)` | `reject` | Roll back a change that adds a new syntax error to a known format. |
| `hidden` | `bool` | `true` | Include hidden files. |
| `gitignore` | `bool` | `false` | Apply the rules of .gitignore files. |
| `binary` | `enum(skip\|text)` | `skip` | Skip binary files or treat them as text. |
| `file_type` | `list<text>` | `[]` | File types by extension, such as go or ts. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `source` | `none` | `effect` |
| `matches` | `match`, `node`, `item` | `effect` |
| `lines` | `line`, `content` | `effect` |

The `effect` record lists one `change` per changed file with the number of
replacements, the lines added and removed and the difference.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

## Examples

Replace the first call in one file:

```nurt example
replace "oldName(" "newName(" in: ["/home/user/app/a.go"] occurrence: 1
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/a.go", "type": "file", "content_utf8": "x := oldName(1)\ny := oldName(2)\n" }
] }
```

Rewrite calls with a regular expression in the Rust files of a directory:

```nurt example
replace /(\w+\([^)]*\))\.unwrap\(\)/ "$1?" in: ["/home/user/app"] glob: "**/*.rs"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/src/main.rs", "type": "file", "content_utf8": "let a = load().unwrap();\n" },
  { "path": "/home/user/app/src/io/read.rs", "type": "file", "content_utf8": "let b = open(p).unwrap();\n" },
  { "path": "/home/user/app/README.md", "type": "file", "content_utf8": "call().unwrap()\n" }
] }
```

Replace a block written with other indentation; the new lines take the
indentation of the file:

```nurt example
replace "start()\nreport()" "prepare()\nstart()\nreport()" in: ["/home/user/app/run.py"]
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/run.py", "type": "file", "content_utf8": "def run():\n    if ready:\n        start()\n        report()\n" }
] }
```

Replace a word in lines 2 and 3 only:

```nurt example
lines "/home/user/notes.txt" 2..3 |> replace "draft" "final"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "draft one\ndraft two\ndraft three\n" }
] }
```

Reformat search matches with their groups:

```nurt example
search /(\w+)=(\S+)/ "/home/user/app/config.ini" |> replace "$1 = $2"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/config.ini", "type": "file", "content_utf8": "port=8080\nhost=example.com\n" }
] }
```

Replace a scheme only when it occurs exactly three times in a directory:

```nurt example
replace "http://" "https://" in: ["/home/user/site"] expect: 3
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/site/a.html", "type": "file", "content_utf8": "http://example.com http://example.com\n" },
  { "path": "/home/user/site/b.html", "type": "file", "content_utf8": "http://example.com\n" }
] }
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_ASSERT` | the number of matches is outside `expect` | extend the pattern, set `occurrence` or change `expect` |
| `E_NO_MATCH` | nothing matches and `on_missing` is `error` | read the current text and give an existing fragment |
| `E_OUT_OF_RANGE` | no file has the occurrence and `on_missing` is `error` | give an `occurrence` from 1 to the number of matches |
| `E_REGEX` | the regular expression or the template is not valid | correct the pattern or pass `template: false` |
| `E_INVALID_VALUE` | the pattern is empty, or `occurrence` is below 1 | give the text to replace and an occurrence from 1 |
| `E_STALE` | a file changed since it was read, or does not have the `if_match` tag | read the file again and repeat the change |
| `E_ENCODING` | the encoding of a file cannot represent the replacement | convert the file with `convert` first |
| `E_BINARY` | a file named by an input record is binary | pass text files only |
| `E_UNSUPPORTED_LANGUAGE` | `scope` or `mode: syntax` has no grammar for a file | use `scope: any` or a literal pattern |
| `E_LIMIT` | the regular expression exceeded the backtracking limit | simplify the pattern |
| `E_ROLLED_BACK` | the change added a syntax error and `validate` is `reject` | correct the new text or pass `validate: warn` |

## Related

`search`, `lines`, `edit`, `insert`, `write`, `diff`, `help skill: "text-editing"`.
