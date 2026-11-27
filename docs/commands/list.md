---
command: list
group: core
summary: List the entries of a directory.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# list

## Synopsis

```text
list <path> [name: value]...
```

## Description

`list` produces one `entry` record for every entry below the directory
`path`, down to `depth` levels. The directory itself is not a record; use
`find` with `depth: 0` for a record of one path. A link to a directory is
listed as the directory it leads to.

Every record carries `depth` (1 for the direct contents) and `relative`, the
path relative to `path` with `/` separators. Names are in NFC; a stored name
that is not valid Unicode or not in NFC is kept in `raw_name`. A name that is
not valid on every supported system carries `portability` with one or more
of `windows_reserved_name`, `windows_invalid_character`,
`windows_trailing_character` and `invalid_unicode`.

A path that is not valid Unicode (on Linux, a name with the byte `0xFF`) is
written with U+FFFD in `path`, `parent` and `relative`; the record then carries
the native bytes in `raw_path`, `raw_parent` and `raw_relative`, and the target
of a link in `raw_link_target`. The next stage reads the entry from `raw_path`,
so `list ... |> remove` removes the entry that was listed.

The order depends on `view`:

| `view` | Order of the records |
|---|---|
| `flat` | by `sort` over all records; ties by path |
| `tree` | depth first, every directory before its contents; the entries of one directory by `sort` |
| `sizes` | as `tree`; every directory carries `total_size` and `file_count` |

`sort: name` compares names in NFC by code points. `sort: type` puts
directories first, then files, links and other entries. `total_size` is the
sum of the sizes of all files and links below the directory, at any depth,
and `file_count` is the number of those files; a file with several hard
links is counted once. With `view: sizes`, `sort: size` orders directories
by their totals.

On a volume that ignores letter case, `path` may differ in case from the
stored name; the listing then starts at the stored spelling, every `path`
field is in that spelling and the notable entry `case_alias` records the
given path.

An empty directory produces no records.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Directory to list. |
| `depth` | `int` | `1` | Number of directory levels to list. |
| `type` | `enum(file\|directory\|link\|any)` | `any` | Kind of entries to list. |
| `hidden` | `bool` | `true` | Include hidden entries. |
| `sort` | `enum(name\|modified\|size\|type)` | `name` | Sort key of the entries. |
| `order` | `enum(asc\|desc)` | `asc` | Sort order. |
| `view` | `enum(flat\|tree\|sizes)` | `flat` | Flat list, tree, or tree with directory size totals. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `entry`, `registry_value` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

Notable entries: `permission_denied_skipped` for directories that cannot be
read, `source_unavailable` for entries that cannot be read for another
reason, `nfd_name` for names stored in another normalization form,
`portability` for names that are not portable and `case_alias` for a path
given in another letter case. Each entry gives the number of entries
concerned and a sample of up to five paths.

## Examples

List the entries of a directory, sorted by name:

```nurt example
list "/home/user/app"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/README.md", "type": "file", "content_utf8": "# app\n" },
  { "path": "/home/user/app/docs/guide.md", "type": "file", "content_utf8": "guide\n" },
  { "path": "/home/user/app/src/main.go", "type": "file", "content_utf8": "package main\n" },
  { "path": "/home/user/app/src/util/util.go", "type": "file", "content_utf8": "package util\n" }
] }
```

Show two levels of the directory as a tree:

```nurt example
list "/home/user/app" depth: 2 view: tree
```

Show the directories with the total size of their contents, largest first:

```nurt example
list "/home/user/app" view: sizes sort: size order: desc
```

List the files of a directory changed most recently first:

```nurt example
list "/home/user/app/src" type: file sort: modified order: desc
```

A response that leaves records out ends with the hint "more: next" and carries the field
`continuation`; the command `next` returns the next part without running the
command again.

```text
matched 312  shown 50  more: next
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path, or list its parent directory. |
| `E_IO` | `path` is not a directory | List the parent directory, or use `find` with `depth: 0`. |
| `E_PERMISSION` | `path` cannot be read | List a directory the account may read. |
| `E_PATH_RELATIVE` | `path` is relative and no base directory is set | Give an absolute path or set the `base` option. |
| `E_INVALID_VALUE` | `depth` is below 1 | Give a depth of 1 or more. |
| `E_UNKNOWN_VALUE` | a value of `type`, `sort`, `order` or `view` is unknown | Use one of the listed values. |
| `E_LIMIT` | the listing exceeds the record limit | Lower `depth`, or use `find` with `glob` or `exclude`. |

## Related

`find` selects entries by name pattern and attributes at any depth; `info`
describes one entry. `help skill: "files"` describes working with files.
