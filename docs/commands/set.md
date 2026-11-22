---
command: set
group: documents
summary: Set values at key paths of a JSON, YAML, TOML, XML, INI, CSV, properties, env or plist document, preserving its layout.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# set

## Synopsis

```text
set <path> [name: value]...
item |> set <value> [name: value]...
```

## Description

`set` changes the values at key paths of a structured document and keeps
every byte outside the changed values: indentation, quoting, comments, key
order, line endings, encoding and byte order mark. A value is written in the
style of its neighbours; a new key is added at the end of its map.

In the source form the document is the file `path`. `key` with `value` sets
one key path; `values` sets several key paths, in the order of the map. All
key paths of one call are changed in one transaction: when any of them
fails, nothing is written. Two key paths of one file that are equal or nested
(`a` and `a.b`) fail with `E_KEY_CONFLICT`, because the second change would
apply to the result of the first; appends (`list[+]`) never conflict.

In the stage form, after `read … key:` or `query`, every input `item`
record names a file, a key path and the entity tag of the file. The value of
each item is set to `value`, which may be an expression of the item.
Items of one file are changed together, in input order.

Key paths use the syntax of the `key` type: `a.b`, `list[0]`, `list[-1]`,
`list[+]` to append, `list[*]` for every element, and `#1:` to select a
document of a multi-document YAML stream. An index equal to the length of a
list appends; a larger index fails with `E_KEY_CONFLICT`. A key path that
matches several nodes changes all of them, and `replacements` counts them.
An implicit repetition, such as a repeated XML element or a duplicated key,
without `[*]` fails with `E_KEY_CONFLICT` and lists the candidates `[0]`,
`[1]`, ….

With `create: true` missing keys and intermediate maps are created and the
change reports the notable event `keys_created`. A missing file is created
when every change of the call sets a value; the new document starts empty,
or with the smallest root map of its format (`{}` for JSON, an empty
`<dict>` for property lists, the first key as the root element for XML).
With `create: false` a missing key fails with `E_KEY_NOT_FOUND`, which lists
the closest existing keys as candidates.

Every change passes two guards of the format: the bytes outside the changed
nodes must stay the same, and the changed document must read back as the
original value with the change applied. A construct that cannot be changed
without losing its layout fails with `E_UNSUPPORTED_CONSTRUCT`, naming the
construct; the file is not changed.

`validate` compares the syntax problems of the document before and after the
change. Only an error that the change introduces counts; errors present
before the change are ignored. With `reject` such an error fails the call,
with `warn` the change is written, a warning is logged and the change gets
the flag `validate_warning`, with `off` the check is skipped.

A call that leaves the document unchanged, for example a value equal to the
current one, writes nothing and reports the notable event `unchanged`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Document to change in the source form. |
| `key` | `key` | none | Key path to set, such as `"scripts.test"`, or `"deps[+]"` to append to a list. |
| `value` | `any` | none | New value at the key path; in the stage form the value of each item. |
| `values` | `map<any>` | none | Several key paths and their values, set in one transaction. |
| `create` | `bool` | `true` | Create missing keys, intermediate maps and a missing file. |
| `format` | `enum(auto\|json\|jsonc\|yaml\|toml\|xml\|ini\|csv\|tsv\|properties\|env\|plist)` | `auto` | Document format; `auto` selects it from the file name and content. |
| `validate` | `enum(off\|warn\|reject)` | `reject` | Treatment of a new syntax error. |
| `if_match` | `text` | none | Expected entity tag of the document, in the form `sha256:<hex>`; a different document is not changed. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `source` | `none` | `effect` |
| `stage` | `item` | `effect` |

The `effect` record lists one change per written file with `action`
(`modify` or `create`), `size_before`, `size_after`, `hash_before`,
`replacements` and `flags`, and the notable events `keys_created`,
`alias_shared`, `merge_override`, `duplicate_key` and `unchanged` reported by
the formats.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

The operation journal preserves every target file before the first write.
Each file is written with the entity tag read during planning as its
expected state; a file changed in the meantime fails with `E_STALE` and no
file is written. `undo` restores every changed file byte for byte and
removes files that the call created.

## Examples

Set one key of a JSON file:

```nurt example
set "/home/user/app/package.json" key: "scripts.test" value: "vitest run"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/package.json", "type": "file", "content_utf8": "{\n  \"name\": \"app\",\n  \"scripts\": {\n    \"test\": \"jest\"\n  }\n}\n" }
] }
```

```text expected
1 file changed
```

Set two keys of a YAML file in one transaction:

```nurt example
set "/home/user/app/compose.yaml" values: {"services.web.image": "nginx:1.27", "services.web.ports[+]": "8443:443"}
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/compose.yaml", "type": "file", "content_utf8": "services:\n  web:\n    image: nginx:1.25\n    ports:\n      - \"8080:80\"\n" }
] }
```

Change the value read by `read … key:`:

```nurt example
read "/home/user/app/config.toml" key: "server.port" |> set 8081
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/config.toml", "type": "file", "content_utf8": "# server settings\n[server]\nport = 8080 # public port\n" }
] }
```

Create a missing file with a nested key:

```nurt example
set "/home/user/app/settings.json" key: "editor.tabSize" value: 2
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_KEY_NOT_FOUND` | a key path does not exist and `create` is `false`; `candidates` lists the closest keys | correct the key path or set `create: true` |
| `E_KEY_CONFLICT` | two key paths of one file overlap, an index is past the end of a list, or a key path matches a repetition without `[*]` | change overlapping keys in separate calls, use `[+]`, or select one candidate |
| `E_STALE` | the document differs from `if_match` or from the entity tag of the item, or changed between planning and writing | read the document again and repeat the change |
| `E_DOC_PARSE` | the document cannot be read, or the change introduces a syntax error with `validate: reject` (`problem`: `validate`) | correct the document or the value, or use `validate: warn` |
| `E_UNSUPPORTED_CONSTRUCT` | the change cannot keep the layout of a construct; `found` names the construct | change the construct by hand, or use `replace` |
| `E_NOT_FOUND` | the file does not exist and `create` is `false`, or a removal names a missing file | check the path or set `create: true` |
| `E_MISSING_PARAM` | neither `key` with `value` nor `values` is given | add `key` and `value`, or `values` |
| `E_TYPE` | `values` is not a map, or the input is not `item` records | pass a map of key paths to values |
| `E_INVALID_VALUE` | `if_match` is not an entity tag, or `validate` has another value | pass the `etag` field of a record |
| `E_UNKNOWN_VALUE` | `format` names a format that is not available | use one of the listed formats |
| `E_LIMIT` | the file is larger than the document size limit | raise `limits.max_file_bytes_text` |

## Related

`read` with `key:` to read values and their entity tags, `query` to select
values with `jq` filters, `remove` to delete the keys of `item` records,
`undo` to revert a change.
