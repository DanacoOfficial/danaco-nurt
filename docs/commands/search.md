---
command: search
group: core
summary: Search file content for text, regular expressions or syntax patterns.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# search

## Synopsis

```text
search <pattern> <path>... [name: value]...
entry|content|line |> search <pattern> [name: value]...
```

## Description

`search` finds a pattern in the content of files. The source form walks the
given files and directories: a directory is searched recursively, with the
filters `glob`, `exclude`, `exclude_preset`, `hidden` and `gitignore`, and
with the file types of `file_type` and `exclude_file_type`. The stage form
searches the files of input `entry` records (a directory entry is walked as
in the source form), the text of input `content` records and the text of
input `line` records. Without matches the result is empty.

`glob` and `exclude` compare patterns and paths in Unicode normalization
form C (NFC). A name stored decomposed, as on macOS volumes, matches a
pattern written composed, and a decomposed pattern matches a composed name.

A pattern is text, a regular expression written as `/…/flags`, or a list of
both. With `mode: auto` text is matched literally and `/…/` is a regular
expression; `mode: literal` and `mode: regex` treat every pattern one way.
With a list of patterns every match carries `pattern`, the index of its
pattern from 0, and `output: counts` gives one count per pattern.

Regular expressions use the syntax of the Rust `regex` crate: classes such
as `[a-z]`, `\d`, `\w`, `\s`, Unicode properties `\p{Greek}`, groups `( )`,
`(?:…)`, named groups `(?<name>…)` and `(?P<name>…)`, and the flags `i`,
`m`, `s` and `x`. Classes, `\w`, `\b` and case folding follow Unicode. `^`
and `$` match at the start and end of every line. Look-ahead, look-behind
and back-references are evaluated by a backtracking engine with a step
limit; such matches report `regex_engine: backtracking`, and a line that
exceeds the step limit counts as not matching (notable
`regex_backtracking`). Compared with JavaScript regular expressions, `\d`
and `\w` also match non-ASCII digits and letters, and the flags `g`, `u`
and `y` do not exist.

Every matching line is one `match` record with the position of its first
match. With `only_matching` every match is one record and `line` holds the
matched text. `start` and `end` count bytes from the start of the file as
stored, also in files that are not UTF-8; `column` counts characters from 1.
With `multiline` a match may cross line breaks and reports `end_line` and
`end_column`; a literal pattern that contains a line break implies it.

Binary files are skipped and counted (notable `binary_skipped`) unless
`binary: text` is given. Text in another encoding, such as UTF-16 or
Windows-1250, is decoded before matching. Lines longer than `max_line`
characters are shortened around the first match, with a marker of the
omitted characters (notable `truncated_lines`).

`output: matches` gives `match` records. `output: counts` gives one `match`
record per file, or per file and pattern, with `count`, the number of
matches, and the position of the first match. `output: files` and
`output: unmatched` give `entry` records of the files with and without
matches, with `match_count`.

`sort: path` keeps the order of the walk (roots in the order given, then
depth first by name) or of the input records, and produces records as files
are searched. `sort: modified` puts the most recently modified files first,
`sort: count` the files with the most matches, and `sort: rank` the files
that match the most patterns, then the most matches, then the shallowest
paths; these orders search every file first. With `adaptive: true` and a
limit on the number of records (for example `|> first 50`), files with the
most matches are shown in full while the records fit in the limit, and the
other files are given as one `match` record with `count` and
`summarized: true`.

`scope`, `enclosing` and `mode: syntax` use the code command group. `scope`
keeps the matches in code, comments or string literals; a file of a
language without a grammar counts as code only. `enclosing` adds
`container`, the qualified name, kind and lines of the innermost definition
around each match; without the code group the field is left out. With
`mode: syntax` the pattern is a code pattern of the language with
metavariables such as `$X`; the captured metavariables are in `named`.

Files are searched in parallel within the thread limit of the execution;
the order of the result does not depend on the number of threads.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `pattern` | `text \| regex \| list<text \| regex>` | required | Pattern; with a list, counts are given per pattern. |
| `path` | `list<path>` | required | Files or directories to search in the source form. |
| `glob` | `glob \| list<glob>` | `"**"` | Name patterns of the files to search. |
| `mode` | `enum(auto\|literal\|regex\|syntax)` | `auto` | Pattern mode; auto treats text as literal and /…/ as a regular expression. |
| `case_sensitive` | `bool` | `true` | Match with case sensitivity. |
| `whole_word` | `bool` | `false` | Match whole words only. |
| `multiline` | `bool` | `false` | Allow matches across lines. |
| `invert` | `bool` | `false` | Produce the lines that do not match. |
| `only_matching` | `bool` | `false` | Give only the matched text instead of the whole line. |
| `trim` | `bool` | `false` | Trim leading and trailing whitespace of lines. |
| `dedupe` | `bool` | `false` | Drop repeated identical lines. |
| `scope` | `enum(any\|code\|comments\|strings)` | `any` | Part of source code in which matches count. |
| `enclosing` | `bool` | `false` | Add the enclosing code element of each match. |
| `max_line` | `int` | `500` | Maximum length of a reported line in characters. |
| `exclude` | `list<glob>` | `[]` | Patterns to exclude, in gitignore syntax. |
| `exclude_preset` | `list<enum(vcs\|deps\|build\|cache)>` | `[]` | Predefined exclusions: version control, dependencies, build output, caches. |
| `context` | `int` | `0` | Lines of context before and after each match. |
| `before` | `int` | none | Lines of context before each match. |
| `after` | `int` | none | Lines of context after each match. |
| `output` | `enum(matches\|files\|counts\|unmatched)` | `matches` | Matches, matching files, counts per file, or files without matches. |
| `max_per_file` | `int` | none | Maximum number of matches per file. |
| `binary` | `enum(skip\|text)` | `skip` | Skip binary files or search them as text. |
| `whitespace` | `enum(exact\|trailing\|indent\|normalize)` | `exact` | Tolerance of whitespace differences for literal patterns. |
| `hidden` | `bool` | `true` | Include hidden files. |
| `gitignore` | `bool` | `false` | Apply the rules of .gitignore files. |
| `language` | `text` | none | Programming language of syntax patterns. |
| `file_type` | `list<text>` | `[]` | File types by extension, such as go or ts; applies in every output mode. |
| `exclude_file_type` | `list<text>` | `[]` | File types by extension to leave out. |
| `sort` | `enum(path\|modified\|count\|rank)` | `path` | Sort key of the results; rank orders by relevance. |
| `adaptive` | `bool` | `false` | Fit the result to the limit: full matches for files with most matches, counts for the rest. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `source` | `none` | `match`, `entry` |
| `stage` | `entry`, `content`, `line` | `match`, `entry` |

`match` fields: `path`, `no`, `column`, `start`, `end`, `text`, `line`,
`etag`, and when they apply `groups` (group 0 is the whole match), `named`,
`before`, `after`, `count`, `pattern`, `container`, `regex_engine`,
`end_line`, `end_column`, `summarized`. In the stage form the offsets of a
`content` record with a range of lines, and of a `line` record without
`start`, count from the start of the record text; consecutive `line` records
of one file are counted together.

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Find literal text in the files of a directory.

```nurt example
search "fn main" "src"
```

```text expected
src/main.rs:1:1: fn main() {
```

Show the names captured by a regular expression, one record per match.

```nurt example
search /fn (?<name>\w+)/ "src" only_matching: true
```

```text expected
src/lib.rs:1:5: fn run
src/main.rs:1:1: fn main
```

Search a log without regard to case, with one line of context.

```nurt example
search "warning" "logs" case_sensitive: false context: 1
```

```text expected
logs/app.log:2:1: WARNING: disk almost full
```

Count the matches of each pattern of a list per file.

```nurt example
search ["run", "main"] "src" output: counts
```

List the files with matches, the files with the most matches first.

```nurt example
search /\brun\b|\bmain\b/ "." output: files sort: count
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | No pattern, or no path in the source form. | Give `pattern` and `path`. |
| `E_REGEX` | The regular expression is not valid; `found` holds the pattern. | Correct the expression at the reported character. |
| `E_LIMIT` | A compiled pattern is too large, more results than `max_records` are held for `sort`, or a file above the text limit is searched with `scope`, `enclosing` or a syntax pattern. | Simplify the pattern, narrow the search, or search the file with a text pattern. |
| `E_UNKNOWN_VALUE` | A value of an enumeration or a file type is unknown. | Use one of the listed values. |
| `E_INVALID_VALUE` | A count is negative, or the pattern list is empty. | Give 0 or more, or at least one pattern. |
| `E_TYPE` | A parameter or an input record has another type. | Pass text, regular expressions, paths, or `entry`, `content` and `line` records. |
| `E_PARAM_CONFLICT` | `invert` is combined with `scope` or `mode: syntax`. | Leave out one of them. |
| `E_UNSUPPORTED` | `scope` or `mode: syntax` is used without the code command group. | Load the code group, or search with `scope: any` and a text pattern. |
| `E_PATH_RELATIVE` | A relative path is given without a base directory. | Give an absolute path or set the `base` option. |
| `E_NOT_FOUND` | A file or directory of `path` does not exist. | Check the path. |
| `E_PERMISSION` | A root cannot be read. | Check the access rights of the path. |
| `E_IO` | A root cannot be read. | Check the access rights of the path. |

## Related

`find`, `read`, `lines`, `replace`, `help skill: "text-editing"`.
