<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Report output

The result of a program is the report `nurt-report/1`. The crate `nurt-report`
reduces the report to the requested level of detail and renders it in one of
four formats. The response envelope `nurt-response/1` carries the reduced
report and the rendered text.

## Call options

| Option | Type | Default | Effect |
|---|---|---|---|
| `format` | `text`, `json`, `markdown`, `compact` | `text` | Format of the rendered result. |
| `verbosity` | `quiet`, `normal`, `detailed` | `normal` | Level of detail of the report. |
| `limit` | integer, 1 to 5000 | `50` | Number of changes or records per page. |
| `diff` | `none`, `stat`, `unified` | `unified` | Form of differences. |
| `diff_context` | integer | `3` | Context lines of a difference; fixed when the hunks are produced, not changed by the renderer. |
| `max_chars` | integer | `0` | Largest number of characters of one text field; `0` means no limit. |
| `fields` | list of names | all fields | Fields kept in the rows of a reading program. |
| `save_to` | absolute path | none | File that receives the full result; the report gives `result_file`. |

Configuration keys `report.diff_max_hunks` (default `20`) and
`report.diff_max_lines` (default `200`) bound the differences at verbosity
`normal`.

The terminal width and the colors apply to the format `text` of the command
line only. The text of a response contains no ANSI sequences and is not
shortened to a terminal width.

## Verbosity

| Level | Kept | Removed |
|---|---|---|
| `quiet` | status, `op`, targets, summary, `notable`, `irreversible`, `error`, `result_file`, records of a reading program | changes, differences, `bulk`, `timing` |
| `normal` | everything of `quiet`, changes up to `limit`, differences up to `report.diff_max_hunks` hunks and `report.diff_max_lines` lines, `bulk` | `timing` |
| `detailed` | everything of `normal`, all differences, `timing` | none |

The number of changes left out is written as a line `… N more changes`; the
header of the differences gives the total number of hunks. A diff mode other
than `unified` removes the differences; the numbers of added and removed lines
stay in the changes.

## Limits

- Changes beyond `limit` are counted, not listed. `targets.listed` is set to
  the number of listed changes.
- Rows of a reading program beyond `limit` are counted, not listed. The cursor
  of the next page is `next_cursor`, set by the engine.
- Differences are limited as given under Verbosity.

## Field shortening

A text field longer than `max_chars` characters is cut at a character
boundary. The record that holds the field gets `truncated: true`. The rule
applies to the records `exec`, `job`, `test`, `analysis`, `git` and to the
header and rows of a reading program, including nested lists, maps and
records.

## Field selection

`fields` keeps only the named fields in each row of a reading program, in the
order of the option. Unknown names are ignored. The header record and the
other records of the report are not affected.

## Formats

### text

One block per part of the report, in a fixed order: corrections, `ran`,
header, targets, program or job block, records, changes, differences, bulk
summary, test, analysis and git records, `notable`, `irreversible`, error,
next cursor, result file, timing. Counts are grouped by a space every three
digits. Sizes use binary units (`b`, `kib`, `mib`, `gib`, `tib`).

```
op 18  replace  ok
targets 3 files · replacements 7 · lines +7 −7 · 41.2kib → 41.3kib
  /srv/app/src/a.go    3 repl  1.1kib → 1.1kib    1a2b3c4d → 9f8e7d6c
diff (3 hunks)
  /srv/app/src/a.go:12
    - func oldName(x int) {
    + func newName(x int) {
notable
  ! many_per_file  /srv/app/src/c/d.go  2
next cursor c_5d2e
```

A reading program writes the header `<kind>  matched N · shown N · next cursor
C` and a table of the rows. A failed program without an operation number
starts with the line `error <code>`. A report with status `ok` and an error
(failure after the effect) writes the effect first and the error block with
the line `effect applied` after it.

The line `result file <path>` appears when the result was written to a file.

### json

The reduced report as one line of JSON that validates against
`nurt-report/1`. The order of fields is fixed. The response has no `rendered`
member for this format, because the report is the result.

### markdown

Headings, pipe tables and fenced blocks for differences and trees. The parts
are the same as in the format `text`. The result file is written as
``result file `<path>` ``.

### compact

One record per line, without headings. The first line is
`report kind=<kind> status=<status>` with `op` and `ran` when present. The
following lines start with the name of the part: `correction`, `targets`,
`summary`, a change line, `diff <path>:<line> <text>`, `bulk`, `notable`,
`irreversible`, `error`, `next cursor=<cursor>`, `result file=<path>`,
`timing`. A value is written as `key=value`; a value that is empty or contains
a space, a quote or `=` is written as a JSON string.

## Tree view

Rows of kind `entry` that all carry an integer `depth` are drawn as a tree in
the formats `text` and `markdown`. A row of depth 0 is a root. A row of depth
`d` is a child of the nearest preceding row of depth `d - 1`. The connectors
are `|-- `, `` `-- `` and `|   `. A directory is written with a trailing `/`,
a file with its size.

```
src/
|-- a.go  1.1kib
`-- b.go  8.0kib
```

The formats `json` and `compact` keep the flat list of rows with their
`depth`.

## Response envelope

| Member | Type | Value |
|---|---|---|
| `nurt` | string | `nurt-response/1` |
| `report` | object | the report after reduction |
| `rendered` | object | `format` and `text` of the rendered report; absent for the format `json` |

`report.next_cursor` is the cursor of the next page.

## Errors

The error of a report is written with the fields `code`, `at`, `fix`, `help`,
`expected`, `actual`, `sample`, `candidates`, `next_steps` and `cause`. Every
code of the error enumeration produces a report that validates against
`nurt-report/1` and renders in all four formats.
