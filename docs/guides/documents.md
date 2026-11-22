---
name: documents
description: Reading and changing JSON, YAML, TOML, XML, INI, CSV and other structured files by key.
commands: [read, set, query, sql, write, diff, find]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# documents

## Key paths

A key addresses a value inside a document. The forms are `a.b` for a map entry, `a[0].b` for a list item, `a[*]` for every item of a list, `a.**` for all descendants, `a["dotted.key"]` for a key that contains a dot, `a[?name == "x"]` for the items that meet a condition and `a[+]` for a new item at the end of a list.

## Reading

`read` with `key` returns the value at a key path. `query` applies a jq filter to a document, a file or a response and returns items that later stages can filter and select.

```nurt
read "/srv/app/values.yaml" key: "image.tag"
query ".dependencies | keys" path: "/srv/app/package.json"
```

## Changing

`set` writes a value at a key path and keeps the layout, comments, key order and style of the file; only the bytes of the changed value change. The format is detected from the file and can be given with `format`. `create: true` adds a missing key, and `values` sets several keys in one call.

```nurt
set "/srv/app/package.json" key: "scripts.test" value: "vitest"
```

## Validation

A write that introduces a new syntax error into a known format is rolled back. `validate: warn` reports the error and keeps the change, and `validate: off` skips the check. The supported formats are `json`, `jsonc`, `yaml`, `toml`, `xml`, `ini`, `csv`, `tsv`, `properties`, `env` and `plist`. A change to a file that was modified after it was read fails with `E_STALE`; see the guide `multi-agent`.

## Databases

`sql` runs statements on an SQLite database file. The database is opened read-only unless `write: true` is given, and a statement that changes data is then recorded in the journal like any other change.

```nurt
sql "/srv/app/app.db" "select id, name from users where active = 1 limit 5"
```

## Errors

`E_KEY_NOT_FOUND` names a key that does not exist and lists the keys at the nearest level. `E_KEY_SYNTAX` marks a malformed key path. `E_DOC_PARSE` means the file is not valid in its format, and `E_FORMAT` means the format could not be determined.
