<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/records.md -->
<!-- Responsibility:   Documents Values and records. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Values and records

Programs exchange typed values and records. This specification defines the value types, their JSON form `nurt-value/1` and every record kind with its fields. The schema `nurt-value-1.schema.json` is generated from the types of the crate `nurt-contracts`; a test of the crate compares the two and validates the golden files of `spec/golden/value` against the schema.

## Value types

| Type | Meaning | Tagged JSON payload | Plain JSON |
|---|---|---|---|
| `null` | absence of a value | none | `null` |
| `bool` | truth value | `true` or `false` | same |
| `int` | signed 64-bit integer; larger integers read from documents are `big_int` with decimal digits | number, or text for `big_int` | number |
| `number` | finite 64-bit floating point number; an exact source text read from a document is `decimal` | number, or text for `decimal` | number |
| `text` | Unicode text | text | text |
| `size` | size in bytes | unsigned integer | integer |
| `duration` | signed duration in milliseconds | integer | integer |
| `instant` | point in time in UTC with nanosecond precision, from 1677 to 2262 | RFC 3339 text with `Z` and 0, 3, 6 or 9 fraction digits | same |
| `date` | date or time read from a document, kept in its source form | `{"kind", "text"}` | text |
| `path` | path in canonical form: `/` as separator, absolute, upper-case drive letter, provider prefixes `registry:`, `env:`, `archive:` | text | text |
| `glob` | file name pattern | text | text |
| `regex` | regular expression | `{"pattern", "flags"}`, flags in the order `imsx`, omitted when empty | `/pattern/flags` |
| `key` | key path in a structured document | text | text |
| `bytes` | binary data | standard base64 with padding | same |
| `range` | inclusive range of integers; both bounds optional, start not greater than end | `{"start", "end"}` | `1..5`, `40..`, `..9`, `5` |
| `enum` | value of an enumeration | text | text |
| `list` | ordered list | array of tagged values | array |
| `map` | map with ordered keys | object of tagged values | object |
| `record` | record of a kind | `{"kind", "fields"}` | object of the fields |

The tagged JSON form of a value is an object with the type in `t` and the payload in `v`, for example `{"t": "size", "v": 1048576}`; `null` is `{"t": "null"}`. The tagged form is lossless and keeps the order of map keys and record fields. The plain form is used for the fields of typed records and for rows in reports; it drops the type tags, which the record kind determines.

## Records

A record has a kind and fields in a fixed order. A field marked as required is always present; any other field is present only when it makes sense or was computed. In the plain JSON form of a record, absent fields are left out, sizes are bytes, durations milliseconds, instants RFC 3339 text in UTC, and fields of type `any` use the tagged form. A typed record of any kind is written as `{"kind": "entry", "record": {...}}`; the dynamic form used by stages that work on fields by name is `{"kind": "entry", "fields": {...}}` with tagged values.

### Paths that are not valid Unicode

A path is text in canonical form, so a path with bytes that are not valid Unicode (a Unix name with the byte `0xFF`) is written with each such byte replaced by U+FFFD, the form for display. The text does not lead back to the entry, so every path field of a record that can hold such a path has a companion field `raw_<field>` of type `bytes` (`raw_path` for `path`, `raw_parent` for `parent`, `raw_from` for `from`). The companion holds the native bytes of the whole path, in the form the system takes: on Unix the bytes of the path, absolute, with `/` separators. It is present only when the path is not valid Unicode; for any other path the record has the display field alone. Only systems whose paths are byte strings (Unix) have the companion fields; a Windows path is always shown in its display form. `raw_name` is older and also covers a name that is valid Unicode but not in NFC.

A command that takes the records of a previous stage reads the path of a record from `raw_path` when the record has it and its `path` is still the display form of it, and from `path` otherwise, so that `find ... |> remove`, `move`, `copy`, `replace` and the other commands act on the entry that was listed. A path written in a program is text and cannot name such an entry; the entry is reached through the records of `list`, `find`, `info` or `search`. The journal keeps the native bytes of the targets of an operation, so `undo` and `redo` restore the entry under its original name. Reports show the display form in the formats `text`, `markdown` and `compact` and give the raw form in the format `json`.

The records of the file system commands (`entry`, `content`, `line`, `match`, `hunk`, `change` and the items of an `operation`) and the records `git_status` (`raw_path`, `raw_renamed_from`) and `git_conflict` (`raw_path`), which hold paths relative to the repository root, follow this rule; records of other families, for example `symbol`, `item` or `cell`, show the display form only. The fields of the report that name a path follow the same rule: `raw_path`, `raw_from`, `raw_to` and `raw_link_target` of [`change`](#change), `raw_path` of [`hunk`](#hunk), of `notable` and of `irreversible` entries, `raw_directory` of the directories of a bulk summary, and `raw_sample_head` and `raw_sample_tail` of its sample (one item for each target of the sample, `null` for a target that is valid Unicode).

| Kind | Description |
|---|---|
| [`entry`](#entry) | File system entry: file, directory, link or another object. |
| [`content`](#content) | Text content of a file, decoded to UTF-8. |
| [`line`](#line) | One line of a file. |
| [`match`](#match) | Match of a search. |
| [`item`](#item) | Value at a key path in a structured document. |
| [`row`](#row) | Row of a table, a CSV file or a query. |
| [`node`](#node) | Node of a syntax tree. |
| [`symbol`](#symbol) | Symbol of source code. |
| [`reference`](#reference) | Reference to a symbol. |
| [`diagnostic`](#diagnostic) | Diagnostic of a compiler, a linter or a language server. |
| [`cell`](#cell) | Cell of a notebook. |
| [`hunk`](#hunk) | Hunk of a unified difference. |
| [`change`](#change) | Change of one file within an effect. |
| [`exec`](#exec) | Result of a finished run of a program. |
| [`job`](#job) | Background job. |
| [`process`](#process) | Process of the operating system. |
| [`port`](#port) | Network port. |
| [`disk`](#disk) | Mounted disk. |
| [`host`](#host) | Description of the device. |
| [`environment_variable`](#environment_variable) | Environment variable; the value is never masked. |
| [`service`](#service) | Service of the operating system. |
| [`package`](#package) | Installed or available package. |
| [`account`](#account) | User or group account. |
| [`log_event`](#log_event) | Event of a system log. |
| [`commit`](#commit) | Commit of a repository. |
| [`git_status`](#git_status) | Status of a path in a repository. |
| [`branch`](#branch) | Branch of a repository. |
| [`schedule`](#schedule) | Scheduled task. |
| [`operation`](#operation) | Entry of the operation journal. |
| [`response`](#response) | Response that an HTTP request received. |
| [`volume`](#volume) | Volume as seen by the journal. |
| [`program_card`](#program_card) | Description of a program. |
| [`archive_entry`](#archive_entry) | Member of an archive. |
| [`test_run`](#test_run) | Test run. |
| [`test_result`](#test_result) | Result of one test. |
| [`finding`](#finding) | Finding of an analysis or a review. |
| [`effect`](#effect) | Effect of a changing or running stage, as a record for the stages after the effect. |
| [`git_effect`](#git_effect) | Effect of an operation on a repository. |
| [`registry_value`](#registry_value) | Value in the Windows registry. |
| [`journal_state`](#journal_state) | State and limits of the operation journal. |
| [`value`](#value) | Result of an expression. |
| [`group`](#group) | Group of records with the same key. |
| [`summary`](#summary) | Statistics of a field. |
| [`field_info`](#field_info) | Field of the records in a pipeline. |
| [`cim_object`](#cim_object) | Management object of Windows. |
| [`blame_line`](#blame_line) | Line annotated with the commit that last changed it. |
| [`hover`](#hover) | Description of a symbol at a position. |
| [`code_action`](#code_action) | Code action offered by a language server. |
| [`language`](#language) | Supported language and its server. |
| [`rename_edit`](#rename_edit) | Planned edit of a rename. |
| [`profile_entry`](#profile_entry) | Entry of a repository profile. |
| [`ping_result`](#ping_result) | Result of a reachability test; times in milliseconds. |
| [`dns_record`](#dns_record) | DNS resource record. |
| [`network_interface`](#network_interface) | Network interface. |
| [`program_location`](#program_location) | Location of a program. |
| [`program_category`](#program_category) | Category of programs. |
| [`schedule_run`](#schedule_run) | One run of a scheduled task. |
| [`analysis`](#analysis) | Verdict of an analysis. |
| [`analyze_tool`](#analyze_tool) | Analysis tool found in a project. |
| [`git_tag`](#git_tag) | Tag of a repository. |
| [`git_worktree`](#git_worktree) | Working tree of a repository. |
| [`git_stash`](#git_stash) | Stash entry. |
| [`git_checkpoint`](#git_checkpoint) | Checkpoint of a working tree. |
| [`git_ref_update`](#git_ref_update) | Update of a reference by a transfer. |
| [`git_conflict`](#git_conflict) | Conflict left by a merge. |
| [`git_trash_entry`](#git_trash_entry) | Preserved copy of a deleted branch or a dropped stash entry. |
| `map` | Record of computed or source-defined fields without a fixed kind, for example the result of `select` with computed fields. |

### entry

File system entry: file, directory, link or another object.

For an entry that does not exist (`exists: false` in the result of `info`), `size` is zero and `modified` is the Unix epoch.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | Absolute path in canonical form. |
| `raw_path` | bytes | no | Native bytes of `path`; present only when the path is not valid Unicode. |
| `name` | text | yes | Last component of the path, in NFC. |
| `stem` | text | no | Name without the last extension. |
| `extension` | text | no | Last extension, without the leading period. |
| `parent` | path | no | Parent directory. |
| `raw_parent` | bytes | no | Native bytes of `parent`; present only when it is not valid Unicode. |
| `type` | enum(file\|directory\|link\|fifo\|device\|other) | yes | Type of the entry. |
| `exists` | bool | no | Whether the entry exists; set in the result of `info`. |
| `size` | size | yes | Size in bytes; for directories as reported by the system. |
| `modified` | instant | yes | Time of the last modification. |
| `created` | instant | no | Time of creation. |
| `accessed` | instant | no | Time of the last access. |
| `mode` | text | no | Permission bits in octal, for example `0644`. |
| `owner` | text | no | Owning account: name and identifier. |
| `group` | text | no | Owning group. |
| `acl` | list<text> | no | Access control entries. |
| `readonly` | bool | no | Read-only. |
| `hidden` | bool | no | Hidden. |
| `link_target` | path | no | Target of a link. |
| `raw_link_target` | bytes | no | Native bytes of `link_target`; present only when it is not valid Unicode. |
| `link_type` | enum(symbolic\|junction\|hard) | no | Kind of a link. |
| `major` | int | no | Major device number of a device node. |
| `minor` | int | no | Minor device number of a device node. |
| `links` | int | no | Number of hard links. |
| `volume` | path | no | Mount point of the volume. |
| `file_id` | text | no | File identifier on the volume. |
| `language` | text | no | Programming or markup language. |
| `mime` | text | no | Media type. |
| `binary` | bool | no | Binary content. |
| `encoding` | text | no | Detected encoding of a text file. |
| `line_ending` | enum(lf\|crlf\|cr\|mixed\|none) | no | Line endings of a text file. |
| `bom` | bool | no | Whether the text starts with a byte order mark. |
| `lines` | int | no | Number of lines. |
| `hash` | text | no | Content hash, for example `sha256:…`. |
| `etag` | text | no | Entity tag of the content at the time of reading. |
| `raw_name` | bytes | no | Name as bytes when it is not valid Unicode or differs from the NFC form. |
| `real_path` | path | no | Path after resolving links. |
| `case_alias` | text | no | Actual spelling on a volume that ignores case. |
| `portability` | list<text> | no | Portability problems of the name, for example `windows_reserved_name`. |
| `depth` | int | no | Depth below the root of the listing. |
| `relative` | text | no | Path relative to the root of the listing. |
| `raw_relative` | bytes | no | Native bytes of `relative`, with `/` separators; present only when it is not valid Unicode. |
| `total_size` | size | no | Total size of a directory tree. |
| `file_count` | int | no | Number of files in a directory tree. |
| `rank` | number | no | Similarity rank of a fuzzy search. |
| `match_count` | int | no | Number of matches in the file. |
| `xattrs` | map<text> | no | Extended attributes. |
| `streams` | list<text> | no | Alternate data streams. |
| `flags` | list<text> | no | File flags such as `immutable`. |
| `details` | map<any> | no | Details requested with `details:`, by kind. |
| `error` | text | no | Error that kept the entry from being read completely. |

### content

Text content of a file, decoded to UTF-8.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `raw_path` | bytes | no | Native bytes of `path`; present only when the path is not valid Unicode. |
| `text` | text | yes | Text, possibly limited to a range. |
| `lines` | int | yes | Number of lines in `text`. |
| `size` | size | yes | Size of the file in bytes. |
| `etag` | text | yes | Entity tag of the content. |
| `encoding` | text | yes | Encoding of the file. |
| `bom` | bool | yes | Whether the file starts with a byte order mark. |
| `line_ending` | enum(lf\|crlf\|cr\|mixed\|none) | yes | Line endings of the file. |
| `range` | range | no | Range of lines in `text`. |
| `ranges` | list<range> | no | Ranges of lines when several were requested. |
| `total_lines` | int | no | Total number of lines of the file. |
| `unchanged` | bool | no | The file did not change since the given entity tag. |
| `truncated` | bool | no | The text was cut at a limit. |
| `binary` | bool | no | Binary content. |
| `image` | object [`ContentImage`](#contentimage) | no | Image content. |
| `page` | int | no | Page of a document, from 1. |
| `pages_total` | int | no | Number of pages of the document. |
| `cells` | list<record [`cell`](#cell)> | no | Cells of a notebook. |
| `symbols` | list<record [`symbol`](#symbol)> | no | Symbols of the outline view. |
| `since_missing` | bool | no | The content given with `since:` was not available. |
| `diff` | list<record [`hunk`](#hunk)> | no | Difference against the content given with `since:`. |
| `decompressed_from` | path | no | Compressed file the text was read from. |
| `view` | text | no | View used, for example `outline`. |
| `revision` | text | no | Full identifier of the repository revision the content was read from. |

### line

One line of a file.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `raw_path` | bytes | no | Native bytes of `path`; present only when the path is not valid Unicode. |
| `no` | int | yes | Line number, from 1. |
| `text` | text | yes | Text without the line ending. |
| `line_ending` | enum(lf\|crlf\|cr\|mixed\|none) | yes | Line ending. |
| `etag` | text | yes | Entity tag of the file. |
| `start` | int | no | Byte offset of the line from the start of the file. |
| `end` | int | no | Byte offset after the line, without the line ending. |
| `range_index` | int | no | Index of the requested range the line belongs to. |
| `encoding` | text | no | Encoding of the file. |

### match

Match of a search.

`start` and `end` are byte offsets from the start of the file; `column` counts Unicode scalar values from 1.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `raw_path` | bytes | no | Native bytes of `path`; present only when the path is not valid Unicode. |
| `no` | int | yes | Line number, from 1. |
| `column` | int | yes | Column of the match, from 1. |
| `start` | int | yes | Byte offset of the match from the start of the file. |
| `end` | int | yes | Byte offset after the match. |
| `text` | text | yes | Matched text. |
| `line` | text | yes | Whole line. |
| `etag` | text | yes | Entity tag of the file. |
| `groups` | list<text> | no | Capture groups by number; index 0 is the whole match and `null` marks a group that did not match. |
| `named` | map<text> | no | Named capture groups. |
| `before` | list<text> | no | Lines before the match. |
| `after` | list<text> | no | Lines after the match. |
| `count` | int | no | Number of matches in the file (`output: counts`). |
| `pattern` | int | no | Index of the pattern in a list of patterns. |
| `container` | object [`MatchContainer`](#matchcontainer) | no | Enclosing symbol. |
| `regex_engine` | enum(linear\|backtracking) | no | Engine that evaluated the pattern. |
| `end_line` | int | no | Last line of a multi-line match. |
| `end_column` | int | no | Column after the match on the last line. |
| `summarized` | bool | no | Only counted, not shown in full. |

### item

Value at a key path in a structured document.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `format` | text | yes | Format of the document, for example `json`. |
| `key` | text | yes | Key path in canonical form. |
| `value` | any | yes | Value. |
| `value_type` | enum(text\|int\|number\|bool\|null\|list\|map\|date\|bytes) | yes | Type of the value in the document. |
| `etag` | text | yes | Entity tag of the file. |
| `line` | int | no | Line, from 1. |
| `column` | int | no | Column, from 1. |
| `end_line` | int | no | Last line. |
| `start` | int | no | Byte offset of the value from the start of the file. |
| `end` | int | no | Byte offset after the value. |
| `comment` | text | no | Comment attached to the value. |
| `document` | int | no | Index of the document in a file with several documents. |
| `anchor` | text | no | Anchor of the value. |
| `alias_of` | text | no | Anchor this value is an alias of. |
| `tag` | text | no | Tag of the value. |

### row

Row of a table, a CSV file or a query.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `no` | int | yes | Row number, from 1. |
| `cells` | map<any> | yes | Cells by column name. |
| `table` | text | no | Table or statement the row comes from. |

### node

Node of a syntax tree. The kind is reserved: the catalog lists `node` among the input kinds of
some commands, but no command of this release produces node records, so a pipeline cannot supply
one.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `language` | text | yes | Language. |
| `kind` | text | yes | Kind of node in the grammar. |
| `start` | int | yes | Byte offset from the start of the file. |
| `end` | int | yes | Byte offset after the node. |
| `line` | int | yes | First line, from 1. |
| `column` | int | yes | First column, from 1. |
| `end_line` | int | yes | Last line. |
| `text` | text | yes | Text of the node. |
| `etag` | text | yes | Entity tag of the file. |
| `end_column` | int | no | Column after the node on the last line. |
| `captures` | map<text> | no | Captured metavariables of the pattern. |

### symbol

Symbol of source code.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name. |
| `kind` | enum(function\|method\|class\|struct\|interface\|enum\|variable\|constant\|field\|module\|type\|macro\|other) | yes | Kind. |
| `path` | path | yes | File. |
| `line` | int | yes | First line, from 1. |
| `column` | int | yes | Column, from 1. |
| `end_line` | int | yes | Last line. |
| `language` | text | yes | Language. |
| `source` | enum(lsp\|syntax) | yes | Source of the information. |
| `etag` | text | yes | Entity tag of the file. |
| `end_column` | int | no | Column after the symbol on the last line. |
| `container` | text | no | Enclosing symbol. |
| `signature` | text | no | Signature. |
| `text` | text | no | Text of the definition. |
| `exported` | bool | no | Exported from its module. |
| `references` | int | no | Number of references. |

### reference

Reference to a symbol.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name of the symbol. |
| `path` | path | yes | File. |
| `line` | int | yes | Line, from 1. |
| `column` | int | yes | Column, from 1. |
| `role` | enum(definition\|reference\|import\|test\|unknown) | yes | Role. |
| `source` | enum(lsp\|syntax) | yes | Source of the information. |
| `etag` | text | yes | Entity tag of the file. |
| `end_column` | int | no | Column after the reference. |
| `text` | text | no | Text of the line. |
| `container` | text | no | Enclosing symbol. |

### diagnostic

Diagnostic of a compiler, a linter or a language server.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `line` | int | yes | Line, from 1. |
| `column` | int | yes | Column, from 1. |
| `severity` | enum(error\|warning\|info\|hint) | yes | Severity. |
| `message` | text | yes | Message. |
| `source` | text | yes | Tool or server that reported it. |
| `code` | text | no | Code of the diagnostic. |
| `end_line` | int | no | Last line. |
| `end_column` | int | no | Column after the fragment on the last line. |
| `language` | text | no | Language. |
| `related` | list<object [`RelatedLocation`](#relatedlocation)> | no | Related positions. |

### cell

Cell of a notebook.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | Notebook file. |
| `index` | int | yes | Position of the cell, from 0. |
| `cell_type` | enum(code\|markdown\|raw) | yes | Type of the cell. |
| `source` | text | yes | Source of the cell. |
| `etag` | text | yes | Entity tag of the notebook file. |
| `id` | text | no | Identifier of the cell. |
| `outputs` | list<any> | no | Outputs as stored in the notebook. |
| `outputs_summary` | text | no | Number and kinds of outputs, without their content. |
| `execution_count` | int | no | Execution counter. |
| `metadata` | map<any> | no | Metadata of the cell. |

### hunk

Hunk of a unified difference.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File after the change. |
| `raw_path` | bytes | no | Native bytes of `path`; present only when the path is not valid Unicode. |
| `old_start` | int | yes | First line before, from 1. |
| `old_len` | int | yes | Number of lines before. |
| `new_start` | int | yes | First line after, from 1. |
| `new_len` | int | yes | Number of lines after. |
| `lines` | list<text> | yes | Lines with the prefixes `+`, `-` and space, and the marker line `\ No newline at end of file`. |
| `id` | text | no | Stable identifier, `path#number`. |
| `header` | text | no | Section heading after `@@`. |
| `old_path` | path | no | File before a rename. |
| `raw_old_path` | bytes | no | Native bytes of `old_path`; present only when it is not valid Unicode. |
| `status` | enum(modified\|added\|deleted\|renamed\|binary\|copied) | no | Status of the file. |
| `binary` | bool | no | Binary content. |
| `old_mode` | text | no | Mode before. |
| `new_mode` | text | no | Mode after. |
| `ignore_whitespace` | bool | no | White space was ignored. |
| `fingerprint` | text | no | First six hexadecimal digits of the SHA-256 hash of the hunk lines; part of the identifier `path#number:fingerprint`. |
| `staged` | bool | no | The hunk is in the index rather than in the worktree. |
| `untracked` | bool | no | The hunk adds an untracked file. |
| `unit` | enum(lines\|words) | no | Unit of the difference. |
| `moved` | list<object [`MovedBlock`](#movedblock)> | no | Blocks of lines moved within the difference. |

#### MovedBlock

| Field | Type | Required | Meaning |
|---|---|---|---|
| `from_path` | path | yes | File the lines were removed from. |
| `from_line` | int | yes | First removed line, from 1. |
| `to_path` | path | yes | File the lines were added to. |
| `to_line` | int | yes | First added line, from 1. |
| `lines` | int | yes | Number of lines. |

### change

Change of one file within an effect.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `raw_path` | bytes | no | Native bytes of `path`; present only when the path is not valid Unicode. |
| `action` | enum(create\|modify\|delete\|rename\|attributes\|link\|truncate\|state) | yes | Action. |
| `from` | path | no | Previous path of a rename. |
| `raw_from` | bytes | no | Native bytes of `from`; present only when it is not valid Unicode. |
| `to` | path | no | New path of a move or a copy. |
| `raw_to` | bytes | no | Native bytes of `to`; present only when it is not valid Unicode. |
| `size_before` | size | no | Size before. |
| `size_after` | size | no | Size after. |
| `hash_before` | text | no | Hash before. |
| `hash_after` | text | no | Hash after. |
| `added` | int | no | Lines added. |
| `removed` | int | no | Lines removed. |
| `replacements` | int | no | Replacements made. |
| `rows_changed` | int | no | Rows inserted, updated or deleted by SQL statements. |
| `flags` | list<text> | no | Notes about the change, for example `crlf_preserved`. |
| `encoding_before` | text | no | Encoding before. |
| `encoding_after` | text | no | Encoding after. |
| `line_ending_before` | enum(lf\|crlf\|cr\|mixed\|none) | no | Line endings before. |
| `line_ending_after` | enum(lf\|crlf\|cr\|mixed\|none) | no | Line endings after. |
| `bom_before` | bool | no | Byte order mark before. |
| `bom_after` | bool | no | Byte order mark after. |
| `link_target` | path | no | Target of a created link. |
| `raw_link_target` | bytes | no | Native bytes of `link_target`; present only when it is not valid Unicode. |
| `store` | text | no | How the previous content was kept. |
| `irreversible` | enum (reason of the effect report) | no | Reason why the change cannot be undone, for example `no_space_for_copy`. |
| `diff` | list<record [`hunk`](#hunk)> | no | Difference of the content. |
| `before` | text | no | State before a change of the action `state`, for example `running`. |
| `after` | text | no | State after a change of the action `state`. |

### exec

Result of a finished run of a program.

`exit_code` is `-1` when the process ended by a signal or a limit; `signal` and `killed_by` tell which.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `command` | list<text> | yes | Arguments as given. |
| `program` | path | yes | Program after resolution in the search path. |
| `cwd` | path | yes | Working directory. |
| `pid` | int | yes | Process identifier. |
| `exit_code` | int | yes | Exit code. |
| `timed_out` | bool | yes | Ended by the time limit. |
| `stalled` | bool | yes | Ended by the idle limit. |
| `started` | instant | yes | Start time. |
| `elapsed` | duration | yes | Time taken. |
| `stdout` | text | yes | Standard output after filters. |
| `stderr` | text | yes | Standard error after filters. |
| `stdout_bytes` | int | yes | Bytes of standard output. |
| `stderr_bytes` | int | yes | Bytes of standard error. |
| `stdout_lines` | int | yes | Lines of standard output. |
| `stderr_lines` | int | yes | Lines of standard error. |
| `decoded_as` | text | yes | Encoding the output was decoded from, for example `utf-8`. |
| `changes` | list<record [`change`](#change)> | yes | Files changed by the run. |
| `tracking` | enum(none\|scan\|events) | yes | How changes were detected. |
| `watch` | list<path> | yes | Directories watched for changes. |
| `launcher` | enum(direct\|cmd) | no | How the program was started. |
| `signal` | enum(terminate\|kill\|interrupt\|hangup\|quit\|abort\|segv\|bus\|fpe\|ill\|pipe\|other) | no | Signal that ended the process. |
| `signal_number` | int | no | Number of that signal. |
| `killed_by` | enum(timeout\|idle\|limit\|stop\|cancel) | no | Who ended the process. |
| `limit_hit` | enum(cpu\|memory) | no | Resource limit reached. |
| `finished` | instant | no | End time. |
| `cpu_time` | duration | no | Processor time of the process group. |
| `max_rss` | size | no | Peak memory of the process group. |
| `shown` | object [`ExecShown`](#execshown) | no | What was shown of the output. |
| `output_file` | path | no | File with the merged output. |
| `stdout_file` | path | no | File with standard output. |
| `stderr_file` | path | no | File with standard error. |
| `output_cursor` | text | no | Cursor for reading further output. |
| `output_sha256` | text | no | Hash of the complete output. |
| `summary_lines` | list<text> | no | Lines matched by `summary:`. |
| `progress` | object [`Progress`](#progress) | no | Last progress. |
| `preimage` | enum(none\|clone\|copy\|partial) | no | Previous content kept before the run. |
| `preimage_entries` | int | no | Number of files kept. |
| `preimage_bytes` | size | no | Bytes kept. |
| `external_effect` | list<text> | no | Effects outside the file system recognised in the arguments. |
| `batches` | list<object [`Batch`](#batch)> | no | Batches of a run over a record stream. |
| `failed_batches` | int | no | Number of failed batches. |
| `env_keys` | list<text> | no | Names of the environment variables set explicitly. |
| `job` | text | no | Job identifier when the run continued in the background. |
| `diagnostics` | object [`ExecDiagnostics`](#execdiagnostics) | no | Errors and warnings of a recognised tool. |
| `digest` | text | no | One line about a successful run: exit code, time, lines and bytes. |

### job

Background job.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `id` | text | yes | Identifier, for example `j7`. |
| `command` | list<text> | yes | Arguments. |
| `cwd` | path | yes | Working directory. |
| `pid` | int | yes | Process identifier. |
| `state` | enum(running\|suspended\|exited\|failed\|stopped\|timed_out\|stalled\|limit\|lost) | yes | State. |
| `started` | instant | yes | Start time. |
| `elapsed` | duration | yes | Time since the start or until the end. |
| `label` | text | no | Label. |
| `description` | text | no | Description. |
| `program` | path | no | Program after resolution. |
| `exit_code` | int | no | Exit code after the end. |
| `signal` | enum(terminate\|kill\|interrupt\|hangup\|quit\|abort\|segv\|bus\|fpe\|ill\|pipe\|other) | no | Signal that ended the process. |
| `killed_by` | enum(timeout\|idle\|limit\|stop\|cancel) | no | Who ended the process. |
| `finished` | instant | no | End time. |
| `progress` | object [`Progress`](#progress) | no | Last progress. |
| `stdout` | text | no | Standard output, when requested. |
| `stderr` | text | no | Standard error, when requested. |
| `stdout_bytes` | int | no | Bytes of standard output. |
| `stderr_bytes` | int | no | Bytes of standard error. |
| `stdout_lines` | int | no | Lines of standard output. |
| `stderr_lines` | int | no | Lines of standard error. |
| `output_file` | path | no | File with the merged output. |
| `stdout_file` | path | no | File with standard output. |
| `stderr_file` | path | no | File with standard error. |
| `output_cursor` | text | no | Cursor for reading further output. |
| `retry_of` | text | no | Job this one repeats. |
| `op` | int | no | Number of the journal operation of the job. |
| `changes` | list<record [`change`](#change)> | no | Files changed, after the end. |
| `session` | text | no | Session that started the job. |
| `suspended_total` | duration | no | Total time suspended. |

### process

Process of the operating system.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `pid` | int | yes | Process identifier. |
| `parent_pid` | int | yes | Parent process identifier. |
| `name` | text | yes | Name. |
| `command` | list<text> | yes | Arguments. |
| `user` | text | yes | Account name. |
| `memory` | size | yes | Resident memory. |
| `started` | instant | yes | Start time. |
| `state` | enum(running\|sleeping\|stopped\|zombie\|idle\|other) | yes | State. |
| `executable` | path | no | Executable file. |
| `uid` | text | no | Account identifier: UID or SID. |
| `cpu` | number | no | Processor use in percent of one core. |
| `threads` | int | no | Number of threads. |
| `cwd` | path | no | Working directory. |
| `children` | list<int> | no | Child processes. |
| `depth` | int | no | Depth in the process tree. |
| `os_raw` | map<any> | no | Raw attributes of the system, outside the stability guarantee. |

### port

Network port.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `protocol` | enum(tcp\|udp) | yes | Protocol. |
| `local` | text | yes | Local address and port, `address:port`. |
| `address` | text | yes | Local address. |
| `port` | int | yes | Local port. |
| `state` | enum(listen\|established\|time_wait\|close_wait\|syn_sent\|syn_received\|fin_wait\|closing\|last_ack\|closed\|none) | yes | State. |
| `remote` | text | no | Remote address and port. |
| `remote_address` | text | no | Remote address. |
| `remote_port` | int | no | Remote port. |
| `pid` | int | no | Owning process. |
| `process` | text | no | Name of the owning process. |
| `family` | enum(ipv4\|ipv6) | no | Address family. |

### disk

Mounted disk.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `mount` | path | yes | Mount point. |
| `device` | text | yes | Device. |
| `filesystem` | text | yes | File system. |
| `total` | size | yes | Total size. |
| `free` | size | yes | Free size available to the account. |
| `used` | size | yes | Used size. |
| `type` | enum(fixed\|removable\|network\|ram\|other) | yes | Type. |
| `case_sensitive` | bool | yes | Names differ by case. |
| `readonly` | bool | yes | Mounted read-only. |
| `label` | text | no | Label. |
| `inodes_total` | int | no | Total number of inodes. |
| `inodes_free` | int | no | Free inodes. |
| `options` | list<text> | no | Mount options. |
| `os_raw` | map<any> | no | Raw attributes of the system, outside the stability guarantee. |

### host

Description of the device.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `os` | enum(linux\|macos\|windows) | yes | Operating system. |
| `os_version` | text | yes | Version of the operating system. |
| `architecture` | text | yes | Processor architecture, for example `x86_64`. |
| `cpus` | int | yes | Logical processors. |
| `memory_total` | size | yes | Total memory. |
| `hostname` | text | yes | Host name. |
| `user` | text | yes | Current account. |
| `home` | path | yes | Home directory. |
| `temp` | path | yes | Directory for temporary files. |
| `timezone` | text | yes | Local time zone. |
| `path_style` | text | yes | Path style: `posix` or `windows`. |
| `nurt_version` | text | yes | Version of this implementation. |
| `features` | list<text> | yes | Features available, for example `reflink` and `journal`. |
| `kernel` | text | no | Kernel version. |
| `memory_free` | size | no | Free memory. |
| `uid` | text | no | Account identifier. |
| `groups` | list<text> | no | Groups of the account. |
| `locale` | text | no | Locale. |
| `uptime` | duration | no | Time since boot. |
| `boot_time` | instant | no | Boot time. |
| `programs` | list<record [`program_location`](#program_location)> | no | Programs found in the search path. |
| `virtualization` | text | no | Virtualisation, when detected. |
| `service` | enum(running\|stopped\|not_installed) | no | State of the background service of the instance. |

### environment_variable

Environment variable; the value is never masked.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name. |
| `value` | text | yes | Value. |
| `scope` | enum(process\|user\|machine) | yes | Scope. |

### service

Service of the operating system.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name. |
| `state` | enum(running\|stopped\|starting\|stopping\|failed\|paused\|unknown) | yes | State. |
| `start_type` | enum(automatic\|manual\|disabled\|on_demand\|static\|unknown) | yes | Start type. |
| `manager` | enum(systemd\|windows\|launchd) | yes | Service manager. |
| `display_name` | text | no | Display name. |
| `state_raw` | text | no | Raw state reported by the manager. |
| `pid` | int | no | Main process. |
| `description` | text | no | Description. |
| `scope` | enum(system\|user) | no | Scope. |
| `path` | path | no | Unit file, property list or program of the service. |
| `account` | text | no | Account the service runs as. |
| `os_raw` | map<any> | no | Raw attributes of the system, outside the stability guarantee. |

### package

Installed or available package.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name. |
| `version` | text | yes | Installed or offered version. |
| `manager` | enum(apt\|dnf\|pacman\|zypper\|brew\|winget\|choco\|scoop\|dpkg\|rpm\|other) | yes | Package manager. |
| `installed` | bool | yes | Installed. |
| `available_version` | text | no | Newer version available. |
| `description` | text | no | Description. |
| `architecture` | text | no | Architecture. |
| `size` | size | no | Installed size. |
| `source` | text | no | Repository. |
| `files_count` | int | no | Number of files. |

### account

User or group account.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name. |
| `kind` | enum(user\|group) | yes | Kind. |
| `id` | text | yes | Identifier: UID, GID or SID. |
| `home` | path | no | Home directory. |
| `shell` | path | no | Login shell. |
| `groups` | list<text> | no | Groups of a user or members of a group. |
| `disabled` | bool | no | Disabled. |
| `full_name` | text | no | Full name. |
| `system` | bool | no | System account. |
| `last_login` | instant | no | Last login. |
| `os_raw` | map<any> | no | Raw attributes of the system, outside the stability guarantee. |

### log_event

Event of a system log.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `time` | instant | yes | Time. |
| `source` | text | yes | Program, provider or subsystem that logged the event. |
| `level` | enum(critical\|error\|warning\|info\|debug) | yes | Level. |
| `message` | text | yes | Message. |
| `id` | text | no | Event identifier. |
| `host` | text | no | Host name. |
| `unit` | text | no | Unit, channel or category. |
| `pid` | int | no | Process identifier. |
| `boot_id` | text | no | Boot identifier. |
| `cursor` | text | no | Position for paging and resuming. |
| `fields` | map<text> | no | Other fields. |

### commit

Commit of a repository.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `id` | text | yes | Full object identifier. |
| `short_id` | text | yes | Abbreviated identifier. |
| `author` | text | yes | Author name. |
| `time` | instant | yes | Author time. |
| `message` | text | yes | Complete message. |
| `subject` | text | yes | First line of the message. |
| `parents` | list<text> | yes | Parent identifiers. |
| `author_email` | text | no | Author e-mail. |
| `committer` | text | no | Committer name. |
| `committer_email` | text | no | Committer e-mail. |
| `commit_time` | instant | no | Commit time. |
| `refs` | list<text> | no | Names of references that point to the commit. |
| `signature` | object [`CommitSignature`](#commitsignature) | no | Signature. |
| `stats` | object [`CommitStats`](#commitstats) | no | Change statistics. |
| `trailers` | map<text> | no | Trailers of the message. |
| `tree` | text | no | Tree identifier. |
| `files` | list<object [`CommitFile`](#commitfile)> | no | Files changed. |
| `branch` | text | no | Branch that received the commit, in the result of `git.commit`. |

### git_status

Status of a path in a repository.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | text | yes | Path in the repository. |
| `raw_path` | bytes | no | Bytes of `path`; present only when it is not valid Unicode. |
| `index` | enum(unmodified\|modified\|added\|deleted\|renamed\|copied\|type_changed\|unmerged\|untracked\|ignored) | yes | State in the index. |
| `worktree` | enum(unmodified\|modified\|added\|deleted\|renamed\|copied\|type_changed\|unmerged\|untracked\|ignored) | yes | State in the working tree. |
| `kind` | enum(tracked\|untracked\|ignored\|conflict) | yes | Kind of entry. |
| `repo` | path | yes | Root of the repository. |
| `renamed_from` | text | no | Path before a rename. |
| `raw_renamed_from` | bytes | no | Bytes of `renamed_from`; present only when it is not valid Unicode. |
| `conflict` | enum(both_modified\|both_added\|both_deleted\|added_by_us\|added_by_them\|deleted_by_us\|deleted_by_them\|rename_rename\|rename_delete\|directory_file\|binary\|submodule) | no | Kind of conflict. |
| `submodule` | object [`Submodule`](#submodule) | no | Status of a submodule. |
| `mode_before` | text | no | Mode before. |
| `mode_after` | text | no | Mode after. |
| `added` | int | no | Lines added. |
| `removed` | int | no | Lines removed. |
| `similarity` | int | no | Similarity of a rename in percent. |

### branch

Branch of a repository.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name. |
| `current` | bool | yes | Checked out in this working tree. |
| `commit` | text | yes | Commit identifier. |
| `remote` | bool | yes | Remote-tracking branch. |
| `upstream` | text | no | Upstream branch. |
| `ahead` | int | no | Commits ahead of the upstream. |
| `behind` | int | no | Commits behind the upstream. |
| `upstream_gone` | bool | no | The upstream no longer exists. |
| `worktree` | path | no | Working tree where the branch is checked out. |
| `subject` | text | no | Subject of the last commit. |
| `time` | instant | no | Time of the last commit. |

### schedule

Scheduled task.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `id` | text | yes | Identifier, for example `s3`. |
| `trigger` | object [`Trigger`](#trigger) | yes | Trigger. |
| `next_run` | instant | yes | Next run. |
| `active` | bool | yes | The task runs. |
| `action` | text | yes | Action: the canonical program text. |
| `label` | text | no | Label. |
| `last_run` | instant | no | Last run. |
| `last_result` | enum(ok\|failed\|timed_out\|skipped_overlap\|missed\|cancelled) | no | Outcome of the last run. |
| `state` | enum(active\|paused\|completed\|expired) | no | State. |
| `runs` | int | no | Runs so far. |
| `max_runs` | int | no | Maximum number of runs. |
| `expires` | instant | no | Expiry time. |
| `on_missed` | text | no | Handling of runs missed while the service was not running. |
| `misfire` | duration | no | Tolerance for a late start. |
| `overlap` | text | no | Handling of a run that starts while the previous one runs. |
| `timeout` | duration | no | Time limit of a run. |
| `created` | instant | no | Creation time. |
| `session` | text | no | Session that created the task. |
| `action_kind` | text | no | Kind of action, for example `pipeline`. |
| `reason` | text | no | Why the task is not active, for example `service_not_running`. |

### operation

Entry of the operation journal.

The record shows no expiry time and nothing from which one could be derived.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `id` | int | yes | Operation number, one sequence per account, from 1. |
| `time` | instant | yes | Commit time. |
| `kind` | text | yes | Command as named in the catalog, for example `replace` or `git.stash`. |
| `action` | text | no | Changing action of a command with several actions, for example `push`. |
| `summary` | text | yes | One-line summary of the effect. |
| `state` | enum(available\|partial\|expired\|undone) | yes | State. |
| `files` | int | yes | Number of items. |
| `bytes` | size | yes | Logical bytes changed. |
| `irreversible` | list<object (irreversible entry of the effect report)> | yes | Effects that cannot be undone; empty when everything can. |
| `bulk` | bool | yes | Several targets in one operation. |
| `session` | text | no | Session of the call. |
| `undo_of` | list<int> | yes | Operations this one undid; empty for other operations. |
| `redo_of` | int | no | Undo operation this one redid. |
| `undo_scope` | enum(op\|to\|file\|paths) | no | Scope of an undo operation. |
| `ast_hash` | text | yes | Hash of the canonical bound program, `sha256:` and 64 hexadecimal digits. |
| `manifest_sha256` | text | yes | Hash of the manifest of the entry. |
| `chain` | text | no | Hash of the manifest of the previous entry of the account; absent for the first entry. |
| `pinned` | bool | yes | Protected from expiry by steps, age and size, but not from the need for space. |
| `items` | list<object [`OperationItem`](#operationitem)> | no | Items of the entry; present only when details are requested, and empty for an expired entry. |

#### OperationItem

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | Target of the item. |
| `raw_path` | bytes | no | Native bytes of `path`; present only when the path is not valid Unicode. |
| `action` | text | yes | Action on the target, the name of the journal target action, for example `create`, `modify` or `rename`. |
| `mode` | text | yes | Preserve mode of the previous content, for example `clone` or `none`. |
| `before_hash` | text | no | Hash of the content before the operation; absent when there was none. |
| `after_hash` | text | no | Hash of the content after the operation; absent when there is none. |
| `undone` | bool | yes | The item has been undone. |
| `released` | bool | yes | The preserved previous content of the item has been released. |

### response

Response that an HTTP request received.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `url` | text | yes | Requested URL. |
| `status` | int | yes | Status code. |
| `headers` | map<any> | yes | Headers by lower-case name; repeated headers joined, `set-cookie` as a list. |
| `body` | text | yes | Body decoded to text. |
| `bytes` | size | yes | Bytes received. |
| `elapsed` | duration | yes | Time taken. |
| `final_url` | text | yes | URL after redirects. |
| `saved` | path | no | File the body was saved to. |
| `content_type` | text | no | Media type. |
| `charset` | text | no | Character set. |
| `truncated` | bool | no | The body was cut. |
| `offset` | int | no | Offset of the body in characters. |
| `total_chars` | int | no | Total characters of the body. |
| `redirects` | list<text> | no | URLs of the redirects. |
| `from_cache` | bool | no | Served from the cache. |
| `unchanged` | bool | no | Not changed since the given time or tag. |
| `etag` | text | no | Entity tag of the response. |
| `last_modified` | instant | no | Last modification reported. |
| `remote_address` | text | no | Address of the remote host. |

### volume

Volume as seen by the journal.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `mount` | path | yes | Mount point. |
| `filesystem` | text | yes | File system. |
| `total` | size | yes | Total size. |
| `free` | size | yes | Free size. |
| `free_without_journal` | size | yes | Free size if the journal held nothing. |
| `reclaimable` | size | yes | Size the journal can release. |
| `reserve` | size | yes | Reserve kept free. |
| `soft` | size | yes | Soft threshold of free space. |
| `journal_bytes` | size | yes | Bytes held by the journal. |
| `journal` | bool | yes | The journal is active on the volume. |
| `journal_reason` | text | no | Why the journal is not active. |

### program_card

Description of a program.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name. |
| `summary` | text | yes | One sentence. |
| `source` | enum(path\|package\|app\|catalog) | yes | Main source. |
| `installed` | bool | yes | Installed. |
| `guide_optional` | bool | yes | Instructions are not required before use. |
| `sources` | list<text> | yes | Sources the card was assembled from. |
| `category` | text | no | Category. |
| `subcategory` | text | no | Subcategory. |
| `path` | path | no | Path. |
| `version` | text | no | Version. |
| `description` | text | no | Description. |
| `guide` | text | no | Instructions of use. |
| `guide_shared` | text | no | Name of instructions shared by several programs. |
| `package` | text | no | Package. |
| `homepage` | text | no | Home page. |
| `license` | text | no | Licence. |
| `aliases` | list<text> | no | Other names that resolve to the program. |
| `verified` | instant | no | Time of the last verification. |
| `tags` | list<text> | no | Tags. |
| `platforms` | list<text> | no | Operating systems. |
| `score` | number | no | Relevance in a search. |

### archive_entry

Member of an archive.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `archive` | path | yes | Archive file. |
| `name` | text | yes | Last component of the member path. |
| `size` | size | yes | Size after extraction. |
| `type` | enum(file\|directory\|link\|fifo\|device\|other) | yes | Type. |
| `path` | text | no | Full path inside the archive. |
| `packed` | size | no | Size in the archive. |
| `modified` | instant | no | Modification time. |
| `mode` | text | no | Permission bits in octal. |
| `link_target` | text | no | Target of a link. |
| `compression` | text | no | Compression method. |
| `encrypted` | bool | no | Encrypted. |
| `crc32` | text | no | CRC-32 checksum. |
| `format` | text | no | Format of the archive. |

### test_run

Test run.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `id` | text | yes | Identifier, for example `run-3`. |
| `framework` | enum(bun\|vitest\|jest\|pytest\|go\|cargo\|npm\|custom) | yes | Framework. |
| `status` | enum(passed\|failed\|error\|timed_out\|no_tests) | yes | Outcome. |
| `passed` | int | yes | Tests passed. |
| `failed` | int | yes | Tests failed. |
| `skipped` | int | yes | Tests skipped. |
| `errors` | int | yes | Tests that could not run. |
| `duration` | duration | yes | Time taken. |
| `command` | list<text> | yes | Arguments of the command. |
| `started` | instant | yes | Start time. |
| `cwd` | path | yes | Working directory. |
| `framework_version` | text | no | Version of the framework. |
| `todo` | int | no | Tests marked as not yet written. |
| `flaky` | int | no | Tests that passed only on retry. |
| `fingerprint` | text | no | Fingerprint of the state of the code. |
| `targets` | list<text> | no | Selected targets. |
| `filter` | text | no | Name filter. |
| `exit_code` | int | no | Exit code of the framework. |
| `output_file` | path | no | File with the complete output. |
| `retry_of` | text | no | Run this one repeats. |
| `op` | int | no | Journal operation of the run. |
| `coverage` | object [`Coverage`](#coverage) | no | Code coverage. |

### test_result

Result of one test.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `run` | text | yes | Test run. |
| `id` | text | yes | Stable identifier, for example `tests/a_test.py::test_x`. |
| `name` | text | yes | Name. |
| `status` | enum(passed\|failed\|skipped\|error\|todo) | yes | Outcome. |
| `suite` | text | no | Suite. |
| `duration` | duration | no | Time taken. |
| `message` | text | no | Failure message. |
| `path` | path | no | File of the test. |
| `line` | int | no | Line, from 1. |
| `column` | int | no | Column, from 1. |
| `stack` | text | no | Stack trace. |
| `stdout` | text | no | Standard output of the test. |
| `stderr` | text | no | Standard error of the test. |
| `retries` | int | no | Number of retries. |

### finding

Finding of an analysis or a review.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `kind` | enum(types\|lint\|format\|complexity\|duplication\|conflict_markers\|whitespace\|large_files\|binary\|secrets\|subject\|other) | yes | Kind. |
| `severity` | enum(error\|warning\|info) | yes | Severity. |
| `message` | text | yes | Message. |
| `line` | int | no | Line, from 1. |
| `column` | int | no | Column, from 1. |
| `end_line` | int | no | Last line. |
| `end_column` | int | no | Column after the fragment on the last line. |
| `tool` | text | no | Tool that reported it. |
| `id` | text | no | Identifier, for example `an-3#17`. |
| `analysis` | text | no | Analysis, for example `an-3`. |
| `rule` | text | no | Rule of the tool, for example `no-unused-vars`. |
| `fixable` | bool | no | The tool can fix it. |
| `sample` | list<text> | no | Lines of the fragment. |
| `value` | number | no | Measured value, for example a complexity. |
| `threshold` | number | no | Threshold exceeded. |
| `related` | list<object [`RelatedLocation`](#relatedlocation)> | no | Related positions, for example the other copy of a duplicate. |
| `hunk` | text | no | Identifier of the hunk of a review. |
| `target` | enum(file\|message) | no | What the finding is about; for a commit message `path` is the root of the repository. |
| `count` | int | no | Number of occurrences aggregated in this finding. |

### effect

Effect of a changing or running stage, as a record for the stages after the effect.

The complete effect is described by the effect report; this record carries the parts that later stages filter and check.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `status` | enum(ok\|planned\|running\|failed\|rolled_back) | yes | Status. |
| `kind` | text | yes | Command, for example `replace`. |
| `op` | int | no | Number of the journal operation. |
| `targets` | int | no | Number of targets. |
| `summary` | map<int> | no | Counters of the summary, for example `files_modified`. |
| `changes` | list<record [`change`](#change)> | no | Changed files. |
| `notable` | list<text> | no | Codes of unusual effects, for example `mass_delete`. |
| `irreversible` | list<object (irreversible entry of the effect report)> | no | Effects that cannot be undone. |
| `exec` | record [`exec`](#exec) | no | Run of a program. |
| `job` | record [`job`](#job) | no | Background job. |
| `test` | record [`test_run`](#test_run) | no | Test run. |
| `analysis` | record [`analysis`](#analysis) | no | Analysis. |
| `target` | text | no | Target of an effect outside the file system, for example `service:nginx`. |
| `action` | text | no | Action on that target, for example `restart`. |
| `before` | any | no | State before. |
| `after` | any | no | State after. |
| `git` | record [`git_effect`](#git_effect) | no | Effect on a repository. |

### git_effect

Effect of an operation on a repository.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `repo` | path | yes | Root of the repository. |
| `state` | enum(completed\|conflicts\|stopped\|fast_forward\|up_to_date\|nothing_to_do) | yes | Outcome. |
| `head_before` | text | no | Commit of `HEAD` before. |
| `head_after` | text | no | Commit of `HEAD` after. |
| `branch` | text | no | Current branch after the operation. |
| `ref_updates` | list<record [`git_ref_update`](#git_ref_update)> | no | Updates of references. |
| `conflicts` | list<record [`git_conflict`](#git_conflict)> | no | Conflicts left in the working tree. |
| `in_progress` | enum(none\|merge\|rebase\|cherry_pick\|revert\|bisect) | no | Operation left in progress. |
| `step` | object [`GitStep`](#gitstep) | no | Progress of an operation with several steps. |
| `objects_written` | int | no | Objects written to the object database. |
| `objects_received` | int | no | Objects received from a remote. |
| `bytes_received` | size | no | Bytes received from a remote. |
| `bytes_sent` | size | no | Bytes sent to a remote. |
| `branch_before` | text | no | Current branch before the operation. |
| `detached` | bool | no | `HEAD` points to a commit, not to a branch. |
| `upstream` | text | no | Upstream branch of the current branch. |
| `ahead` | int | no | Commits of the current branch not in the upstream. |
| `behind` | int | no | Commits of the upstream not in the current branch. |
| `upstream_gone` | bool | no | The configured upstream no longer exists. |
| `merge_head` | text | no | Commit being merged while a merge is in progress. |
| `counts` | object [`GitCounts`](#gitcounts) | no | Numbers of changed paths by state. |
| `index` | object [`GitIndexSummary`](#gitindexsummary) | no | Summary of the index after staging. |
| `checkpoint` | text | no | Checkpoint created by the operation. |
| `checkpoint_before` | text | no | Checkpoint saved before a restore or a switch. |
| `state_token` | text | no | Hash of the repository state for the `expect_state` parameter of a later call. |

#### GitCounts

| Field | Type | Required | Meaning |
|---|---|---|---|
| `staged` | int | yes | Paths with changes in the index. |
| `unstaged` | int | yes | Paths with changes in the worktree not in the index. |
| `untracked` | int | yes | Untracked paths. |
| `conflicts` | int | yes | Paths with conflicts. |

#### GitIndexSummary

| Field | Type | Required | Meaning |
|---|---|---|---|
| `files` | int | yes | Files with staged changes. |
| `added` | int | yes | Lines added. |
| `removed` | int | yes | Lines removed. |

### registry_value

Value in the Windows registry.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | Key, for example `registry:/HKCU/Software/Example`. |
| `name` | text | yes | Name; empty for the default value of the key. |
| `type` | enum(string\|expand_string\|multi_string\|dword\|qword\|binary\|none\|other) | yes | Type. |
| `value` | any | yes | Value: text, int, list of texts or bytes. |
| `size` | size | yes | Size in bytes. |
| `view` | enum(64\|32) | no | Registry view. |

### journal_state

State and limits of the operation journal.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `active` | bool | yes | The journal is active. |
| `entries` | int | yes | Number of entries. |
| `bytes` | size | yes | Bytes held. |
| `limits` | list<object [`JournalLimit`](#journallimit)> | yes | Active limits with their sources. |
| `volume` | path | no | Volume, when one was selected. |
| `oldest` | instant | no | Time of the oldest entry. |
| `oldest_id` | int | no | Number of the oldest entry. |
| `newest_id` | int | no | Number of the newest entry. |
| `pinned` | int | no | Pinned entries. |
| `reason` | text | no | Why the journal is not active. |

### value

Result of an expression.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `value` | any | yes | Value. |
| `type` | text | yes | Language type of the value. |

### group

Group of records with the same key.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `key` | any | yes | Key: one value, or a map for several keys. |
| `count` | int | yes | Number of records. |
| `items` | list<any> | no | The records, when requested. |

### summary

Statistics of a field.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `count` | int | yes | Number of records. |
| `field` | text | no | Field. |
| `group` | any | no | Group key. |
| `sum` | any | no | Sum. |
| `min` | any | no | Smallest value. |
| `max` | any | no | Largest value. |
| `average` | any | no | Average. |

### field_info

Field of the records in a pipeline.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `field` | text | yes | Field name. |
| `type` | text | yes | Language type. |
| `example` | any | no | Example value. |
| `required` | bool | no | Present in every record. |
| `records` | int | no | Number of records that have it. |

### cim_object

Management object of Windows.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `class` | text | yes | Class. |
| `path` | text | yes | Object path. |
| `properties` | map<any> | yes | Properties. |

### blame_line

Line annotated with the commit that last changed it.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | text | yes | File. |
| `no` | int | yes | Line number, from 1. |
| `commit` | text | yes | Commit identifier. |
| `short_id` | text | yes | Abbreviated identifier. |
| `author` | text | yes | Author. |
| `time` | instant | yes | Author time. |
| `text` | text | yes | Text of the line. |
| `author_email` | text | no | Author e-mail. |
| `original_path` | text | no | Path in that commit. |
| `original_no` | int | no | Line number in that commit. |
| `boundary` | bool | no | The commit is a boundary of the history. |

### hover

Description of a symbol at a position.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `line` | int | yes | Line, from 1. |
| `column` | int | yes | Column, from 1. |
| `source` | enum(lsp\|syntax) | yes | Source of the information. |
| `signature` | text | no | Signature. |
| `documentation` | text | no | Documentation. |

### code_action

Code action offered by a language server.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `line` | int | yes | Line, from 1. |
| `title` | text | yes | Title. |
| `source` | enum(lsp\|syntax) | yes | Source of the information. |
| `end_line` | int | no | Last line. |
| `kind` | text | no | Kind, for example `quickfix`. |

### language

Supported language and its server.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `language` | text | yes | Language. |
| `extensions` | list<text> | yes | File extensions. |
| `server_found` | bool | yes | A server was found. |
| `grammar_version` | text | no | Version of the built-in grammar. |
| `server` | text | no | Server program. |
| `server_running` | bool | no | The server runs. |
| `server_pid` | int | no | Process of the server. |
| `server_root` | path | no | Root directory of the server. |

### rename_edit

Planned edit of a rename.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `line` | int | yes | Line, from 1. |
| `column` | int | yes | Column, from 1. |
| `end_line` | int | yes | Last line. |
| `end_column` | int | yes | Column after the fragment. |
| `old_text` | text | yes | Text before. |
| `new_text` | text | yes | Text after. |
| `kind` | enum(code\|comment\|string\|import) | yes | Kind of text. |
| `source` | enum(lsp\|syntax) | yes | Source of the information. |

### profile_entry

Entry of a repository profile.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `section` | enum(commands\|entry_points\|hotspots\|manifests) | yes | Section. |
| `name` | text | yes | Name. |
| `path` | path | no | File. |
| `command` | text | no | Command. |
| `origin` | text | no | Where the entry comes from, for example a manifest. |
| `changes` | int | no | Number of changes in the history. |
| `last_change` | instant | no | Time of the last change. |

### ping_result

Result of a reachability test; times in milliseconds.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `host` | text | yes | Host as given. |
| `address` | text | yes | Address used. |
| `sent` | int | yes | Requests sent. |
| `received` | int | yes | Replies received. |
| `loss` | number | yes | Loss in percent. |
| `rtt_min` | number | no | Shortest round trip. |
| `rtt_avg` | number | no | Average round trip. |
| `rtt_max` | number | no | Longest round trip. |
| `rtt_stddev` | number | no | Standard deviation of the round trip. |

### dns_record

DNS resource record.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name. |
| `type` | text | yes | Type, for example `AAAA`. |
| `value` | text | yes | Data. |
| `ttl` | duration | yes | Time to live. |
| `server` | text | no | Server that answered. |

### network_interface

Network interface.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name. |
| `addresses` | list<text> | yes | Addresses with prefix length. |
| `up` | bool | yes | The interface is up. |
| `mac` | text | no | Hardware address. |
| `mtu` | int | no | Maximum transmission unit. |
| `kind` | enum(ethernet\|wireless\|loopback\|virtual\|other) | no | Kind. |
| `gateway` | text | no | Default gateway. |

### program_location

Location of a program.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name. |
| `found` | bool | yes | The program was found. |
| `path` | path | no | Path that the name resolves to. |
| `version` | text | no | Version. |
| `source` | enum(path\|package\|app\|catalog) | no | Source. |
| `all_paths` | list<path> | no | Every match in the search path, in order of resolution. |

### program_category

Category of programs.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name. |
| `count` | int | yes | Number of programs. |
| `source` | text | yes | Source. |
| `title` | text | no | Title. |
| `description` | text | no | Description. |
| `subcategories` | list<text> | no | Subcategories. |

### schedule_run

One run of a scheduled task.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `schedule_id` | text | yes | Scheduled task. |
| `started` | instant | yes | Start time. |
| `exit_state` | enum(ok\|failed\|timed_out\|skipped_overlap\|missed\|cancelled) | yes | Outcome. |
| `trigger_time` | instant | yes | Planned time. |
| `elapsed` | duration | no | Time taken. |
| `operation` | int | no | Journal operation of the run. |
| `message` | text | no | Message. |

### analysis

Verdict of an analysis.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `id` | text | yes | Identifier, for example `an-3`. |
| `passed` | bool | yes | The analysis passed. |
| `fail_on` | text | yes | Lowest severity that fails the analysis. |
| `errors` | int | yes | Errors found. |
| `warnings` | int | yes | Warnings found. |
| `infos` | int | yes | Information found. |
| `checks` | list<object [`AnalysisCheck`](#analysischeck)> | yes | Checks performed. |
| `paths` | list<path> | yes | Analysed paths. |
| `changed_only` | bool | yes | Only changed files were analysed. |
| `started` | instant | yes | Start time. |
| `duration` | duration | yes | Time taken. |
| `fingerprint` | text | no | Fingerprint of the state of the code. |
| `duplication_percent` | number | no | Share of duplicated lines in the analysed files, in percent. |
| `op` | int | no | Journal operation of the analysis. |

### analyze_tool

Analysis tool found in a project.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `tool` | text | yes | Tool. |
| `language` | text | yes | Language. |
| `check` | enum(types\|lint\|format) | yes | Check performed. |
| `found` | bool | yes | The tool was found. |
| `version` | text | no | Version. |
| `config` | path | no | Configuration file. |
| `command` | list<text> | no | Command. |

### git_tag

Tag of a repository.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name. |
| `target` | text | yes | Identifier of the tagged object. |
| `kind` | enum(lightweight\|annotated) | yes | Kind. |
| `tagger` | text | no | Tagger. |
| `time` | instant | no | Time. |
| `message` | text | no | Message. |
| `signature` | object [`CommitSignature`](#commitsignature) | no | Signature. |

### git_worktree

Working tree of a repository.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | Directory. |
| `head` | text | yes | Commit identifier of `HEAD`. |
| `detached` | bool | yes | `HEAD` is detached. |
| `bare` | bool | yes | Bare repository. |
| `main` | bool | yes | The main working tree. |
| `branch` | text | no | Branch. |
| `locked` | text | no | Lock reason. |
| `prunable` | bool | no | Can be pruned. |

### git_stash

Stash entry.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `index` | int | yes | Position, 0 for the newest. |
| `id` | text | yes | Commit identifier. |
| `message` | text | yes | Message. |
| `time` | instant | yes | Time. |
| `includes_untracked` | bool | yes | Untracked files included. |
| `branch` | text | no | Branch the entry was made on. |

### git_checkpoint

Checkpoint of a working tree.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `id` | text | yes | Identifier, for example `cp-2`. |
| `commit` | text | yes | Commit identifier. |
| `time` | instant | yes | Time. |
| `ref` | text | yes | Reference, for example `refs/nurt/checkpoints/cp-2`. |
| `index_included` | bool | yes | The index was included. |
| `label` | text | no | Label. |
| `files` | int | no | Number of files. |

### git_ref_update

Update of a reference by a transfer.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `ref` | text | yes | Local reference. |
| `status` | enum(new\|updated\|forced\|deleted\|up_to_date\|rejected\|error) | yes | Outcome. |
| `remote` | text | yes | Remote. |
| `remote_ref` | text | no | Remote reference. |
| `old` | text | no | Identifier before. |
| `new` | text | no | Identifier after. |
| `reason` | text | no | Reason of a rejection, for example `non_fast_forward`. |

### git_conflict

Conflict left by a merge.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | text | yes | Path in the repository. |
| `raw_path` | bytes | no | Bytes of `path`; present only when it is not valid Unicode. |
| `kind` | enum(both_modified\|both_added\|both_deleted\|added_by_us\|added_by_them\|deleted_by_us\|deleted_by_them\|rename_rename\|rename_delete\|directory_file\|binary\|submodule) | yes | Kind. |
| `markers` | bool | yes | The file contains conflict markers. |
| `base` | text | no | Object identifier of the base. |
| `ours` | text | no | Object identifier of the current side. |
| `theirs` | text | no | Object identifier of the other side. |

### git_trash_entry

Preserved copy of a deleted branch or a dropped stash entry.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name of the deleted branch or stash entry. |
| `kind` | enum(branch\|stash) | yes | Kind. |
| `ref` | text | yes | Reference that keeps the copy, for example `refs/nurt/trash/feature`. |
| `commit` | text | yes | Commit identifier. |
| `time` | instant | yes | Time of the deletion. |
| `subject` | text | no | First line of the message of the commit. |

## Nested objects

### ContentImage

Image carried by a content record.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `mime` | text | yes | Media type, for example `image/png`. |
| `width` | int | no | Width in pixels. |
| `height` | int | no | Height in pixels. |
| `data` | bytes | no | Image data. |

### MatchContainer

Symbol enclosing a match.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Name of the symbol. |
| `kind` | enum(function\|method\|class\|struct\|interface\|enum\|variable\|constant\|field\|module\|type\|macro\|other) | yes | Kind of the symbol. |
| `line` | int | yes | First line, from 1. |
| `end_line` | int | no | Last line. |

### RelatedLocation

Related position in a file.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `line` | int | yes | Line, from 1. |
| `end_line` | int | no | Last line. |
| `message` | text | no | Explanation. |

### OutputShown

How much of one output stream is shown.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `head` | int | yes | Lines shown from the start. |
| `tail` | int | yes | Lines shown from the end. |
| `omitted_lines` | int | yes | Lines left out between them. |
| `filtered` | int | yes | Lines removed by output filters. |

### ExecShown

How much of the output of a run is shown.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `stdout` | object [`OutputShown`](#outputshown) | no | Standard output. |
| `stderr` | object [`OutputShown`](#outputshown) | no | Standard error. |

### ExecDiagnostic

Error found in the output of a known tool; every field is one line.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `file` | text | no | File named by the tool. |
| `line` | int | no | Line, from 1. |
| `col` | int | no | Column, from 1. |
| `code` | text | no | Code of the error, for example `E0308` or `TS2322`. |
| `message` | text | yes | Message. |
| `severity` | enum(error\|warning\|info\|hint) | yes | Severity. |
| `count` | int | no | Places the same error was reported at, when more than one. |

### ExecDiagnostics

Errors and warnings found in the output of a known tool.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `tool` | text | yes | Tool whose format was recognised: `cargo`, `pytest`, `tsc`, `gcc` (gcc or clang) or `go`. |
| `errors` | int | yes | Number of errors in the output. |
| `warnings` | int | yes | Number of warnings in the output. |
| `first` | list<object [`ExecDiagnostic`](#execdiagnostic)> | yes | The first distinct errors, at most 10. |

### Progress

Progress of a run.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `percent` | number | no | Percentage, when the output shows one. |
| `line` | text | yes | Last progress line. |

### Batch

One batch of a run over a record stream.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `no` | int | yes | Batch number, from 1. |
| `args` | list<text> | yes | Arguments of the batch. |
| `exit_code` | int | yes | Exit code. |
| `elapsed` | duration | yes | Time taken. |

### CoverageFile

Coverage of one file.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | path | yes | File. |
| `lines_pct` | number | yes | Percentage of covered lines. |
| `uncovered` | list<range> | no | Ranges of uncovered lines. |

### Coverage

Code coverage of a test run.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `lines_pct` | number | yes | Percentage of covered lines. |
| `branches_pct` | number | no | Percentage of covered branches. |
| `functions_pct` | number | no | Percentage of covered functions. |
| `statements_pct` | number | no | Percentage of covered statements. |
| `files` | list<object [`CoverageFile`](#coveragefile)> | no | Coverage per file. |
| `report_file` | path | no | Report file written by the framework. |
| `format` | enum(lcov\|cobertura\|istanbul\|go\|llvm) | no | Format of the report file. |

### AnalysisCheck

One check of an analysis.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `check` | text | yes | Check, for example `lint`. |
| `tool` | text | no | Tool that performed it. |
| `status` | enum(ok\|failed\|skipped\|missing) | yes | Outcome. |
| `findings` | int | no | Number of findings. |
| `duration` | duration | no | Time taken. |

### Trigger

Trigger of a scheduled task.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `kind` | enum(cron\|every\|at) | yes | Kind of trigger. |
| `expression` | text | yes | Cron expression, interval or point in time as written. |
| `timezone` | text | no | Time zone of the trigger. |

### CommitFile

File changed by a commit.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | text | yes | Path in the repository. |
| `status` | enum(unmodified\|modified\|added\|deleted\|renamed\|copied\|type_changed\|unmerged\|untracked\|ignored) | yes | Change. |
| `old_path` | text | no | Path before a rename. |
| `added` | int | no | Lines added. |
| `removed` | int | no | Lines removed. |
| `binary` | bool | no | Binary content. |

### CommitSignature

Signature of a commit or a tag.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `status` | enum(none\|good\|bad\|unknown_key\|unsupported) | yes | Verification status. |
| `format` | enum(openpgp\|ssh\|x509) | no | Format. |
| `key` | text | no | Key identifier or fingerprint. |
| `signer` | text | no | Signer. |

### CommitStats

Change statistics of a commit.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `files` | int | yes | Files changed. |
| `added` | int | yes | Lines added. |
| `removed` | int | yes | Lines removed. |

### Submodule

Status of a submodule.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `commit_changed` | bool | yes | The recorded commit changed. |
| `modified` | bool | yes | Tracked content modified. |
| `untracked` | bool | yes | Untracked content present. |

### JournalLimit

Active limit of the journal.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | Limit: `steps`, `age` or `size`. |
| `value` | any | yes | Value, for example an int, a duration or a size; `unlimited` as an enumeration value. |
| `source` | text | yes | Where the value comes from: `default`, `system`, `user`, `project`, `session` or `floor`. |
| `scope` | text | no | Scope of a value set during work: `session` or `user`. |
| `floor` | any | no | Protection floor that the value cannot go below. |
| `expires` | int | no | Entries that expire because of a lowered limit. |

### GitStep

Progress of an operation with several steps, such as a rebase.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `done` | int | yes | Steps done. |
| `total` | int | yes | Steps in total. |

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
