---
name: encodings
description: Detecting, preserving and converting text encodings and line endings.
commands: [info, read, write, convert, find, filter, select]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# encodings

## Detection

Text files are read in the encoding that Nurt detects: a byte order mark, valid UTF-8 or a code page. The `entry` record of `info` and `find` has the fields `encoding`, `line_ending` and `binary`, so a set of files can be inspected before any change.

```nurt
find "/srv/app" glob: "**/*.txt" |> select path encoding line_ending
```

## Preservation

A change to a text file keeps its encoding, its line endings and its byte order mark. Output of commands is always UTF-8. `read` with `encoding` forces the encoding when the detection is wrong, and `write` with `encoding`, `line_ending` and `bom` sets them for a new file.

```nurt
read "/srv/legacy/readme.txt" encoding: "windows-1250"
write "/srv/app/notes.txt" "first\nsecond\n" line_ending: crlf bom: true
```

## Conversion

`convert` changes the encoding, the line endings or the byte order mark of files. A parameter that is not given leaves that property unchanged, and `line_ending: keep` states this explicitly. The conversion is recorded in the journal and can be undone.

```nurt
convert "/srv/legacy/readme.txt" encoding: "utf-8" line_ending: lf
find "/srv/app" glob: "**/*.txt" |> filter line_ending == "crlf" |> convert line_ending: lf
```

## Mixed and invalid content

A file that mixes line endings keeps each line ending unless `convert` is applied. A byte sequence that is not valid in the encoding fails with `E_ENCODING` and the position of the first invalid byte. A file that contains binary data is skipped by `search` and `replace` unless `binary: text` is given.

## Errors

`E_ENCODING` reports an invalid sequence or an unsupported encoding name. `E_BINARY` reports that a text command met binary content. `E_UNSUPPORTED` reports an encoding that Nurt does not convert.
