---
name: text-editing
description: Reading, searching, replacing, inserting and removing text in files.
commands: [read, lines, search, replace, insert, edit, remove, write, assert]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# text-editing

## Reading ranges

`read` with `lines` returns a range of lines, `last` returns the end of a file and `around` with `context` returns the lines around a position. `lines` produces one `line` record per line, so later stages can filter them.

```nurt
read "/srv/app/main.go" lines: 10..40
lines "/srv/app/main.go" |> filter text ~ /^func /
```

## Searching

`search` takes a pattern and a path. `mode` selects `literal`, `regex`, `syntax` or `auto`, `case_sensitive` and `whole_word` refine the match, `context`, `before` and `after` add surrounding lines, and `output` returns matches, file names, counts or the files without a match.

```nurt
search "deprecated" "/srv/app/src" glob: "**/*.go" context: 2
search /oldName\(/ "/srv/app/src" glob: "**/*.go" output: files
```

## Replacing

`replace` changes text in the files of a search or in the files named by `in`. `occurrence` limits the change to one occurrence, `whitespace` selects how whitespace is compared and `template` enables the groups `$1`, `$2` in the replacement. `expect` stops the change with `E_ASSERT` unless the number of replacements is the given one.

```nurt
search /oldName\(/ "/srv/app/src" glob: "**/*.go" |> assert paths: 12 |> replace "newName("
replace "colour" "color" in: "/srv/app/docs/a.md" expect: 3
```

## Inserting and removing

`insert` adds text before or after a line, a match or a node, or at the start or end of a file. `remove` applied to `line` or `match` records deletes them. Both keep the encoding and line endings of the file.

```nurt
insert "import \"fmt\"\n" in: "/srv/app/main.go" line: 3 position: after
search "debug_print" "/srv/app/src" glob: "**/*.go" |> remove
```

## Several changes to one file

`edit` applies a list of changes to one file as a single write: all of them are applied or none. A change that does not match fails with `E_NO_MATCH` and the file stays as it was. `write` with `mode: append` or `mode: prepend` adds text without reading the file first.
