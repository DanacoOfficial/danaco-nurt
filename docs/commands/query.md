---
command: query
group: documents
summary: Select and transform document values with a jq filter.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# query

## Synopsis

```text
content|entry|item|response |> query <expression>
query <expression> path: <path>
```

## Description

`query` applies a filter in `jq` syntax to documents and produces one `item`
record per result. Filters are compiled and run in-process; the `jq` program
is not needed.

In the source form the document is the file `path`. In the stage form every
input record supplies the value given to the filter:

| Input | Value given to the filter |
|---|---|
| `content` | the text read as a document of its format; JSON when no format is recognised |
| `entry` | the file of the entry read as a document, from `raw_path` when the entry has it (a name that is not valid Unicode); entries other than files are refused |
| `item` | the value of the item; result keys extend the key of the item |
| `response` | the body read as a document; JSON when the content type names JSON or no format is recognised |

Every supported document format is read into the same value model: maps,
lists, text, numbers, booleans and null. Dates and times are text in RFC 3339
form, binary data is Base64 text, and numbers keep their exact digits when
the binary form would change them. A multi-document YAML stream gives one
value per document.

A result that the filter selects by a path, such as `.spec.replicas` or
`.items[]`, gets the canonical key path of its position in the document as
`key`, so that `set` and `remove` can change it. Other results, such as
`length` or an object built by the filter, get the key `"."`. The identity
filter on a whole document gives the key `$`. `value_type` is the type of the
result value; `path`, `format` and `etag` are those of the input.

A filter given as a constant is compiled when the program is bound, before
any document is read. A filter that does not compile fails with `E_QUERY`
and the line and column of the problem in `details`. A runtime error of the
filter, including `error(…)`, fails with `E_QUERY` after the results produced
before it, with the file in `at`. `halt` ends the filter; `halt_error` fails
with `E_QUERY` and the text written by the filter.

`env` and `$ENV` read the environment of the session, `now` reads the clock
of the session, and `debug` and `stderr` write log events. `input`,
`inputs` and modules are not available.

The filter language follows `jq` 1.8.1. A string multiplied by a number, a
deletion of several paths, an update below `null` or past the end of a list, a
fractional list index, the index of an object by a number, the slice of `null`,
`@base64d`, `repeat` and `fromstream` give the results of `jq` 1.8.1. Two
results differ from it:

| Case | `query` | `jq` 1.8.1 |
|---|---|---|
| `captures` of `match` | groups that did not take part are omitted | groups that did not take part are `null` |
| `@base32`, `@base32d` | the filter does not compile | runtime error |

An update that extends a list, and a string made by multiplying a string, are
limited in size by `limits.max_file_bytes_text`; a larger result fails with
`E_LIMIT`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `expression` | `text` | required | Filter in jq syntax. |
| `path` | `path` | none | Document to query in the source form. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `stage` | `content`, `entry`, `item`, `response` | `item` |
| `source` | `none` | `item` |

Results are produced while the filter runs. A pipeline that uses only the
first results stops the filter.

## Effect

| Property | Value |
|---|---|
| Category | `transform` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

List the dependency names of a package manifest:

```nurt example
query ".dependencies | keys[]" path: "/home/user/app/package.json"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/package.json", "type": "file", "content_utf8": "{\n  \"name\": \"app\",\n  \"dependencies\": {\n    \"react\": \"^19.0.0\",\n    \"zod\": \"^3.24.0\"\n  }\n}\n" }
] }
```

Select the images of a YAML file read by `read`:

```nurt example
read "/home/user/app/compose.yaml" |> query ".services[].image"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/compose.yaml", "type": "file", "content_utf8": "services:\n  web:\n    image: nginx:1.27\n  db:\n    image: postgres:17\n" }
] }
```

```text expected
nginx:1.27
```

Raise every replica count by one through the keys of the results:

```nurt example
query ".deployments[] | select(.replicas < 3) | .replicas" path: "/home/user/app/scale.json" |> set (value + 1)
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/scale.json", "type": "file", "content_utf8": "{\"deployments\": [{\"name\": \"api\", \"replicas\": 2}, {\"name\": \"worker\", \"replicas\": 5}]}\n" }
] }
```

Count the sections of a TOML file:

```nurt example
query "[.[] | objects] | length" path: "/home/user/app/config.toml"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/config.toml", "type": "file", "content_utf8": "[server]\nport = 8080\n\n[database]\nurl = \"postgres://localhost/app\"\n" }
] }
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_QUERY` | the filter does not compile (`problem`: `syntax`, `undefined`, `module`), fails while it runs (`runtime`), or ends with `halt_error` (`halt`) | correct the filter; `details` holds the line and column of a compile error |
| `E_DOC_PARSE` | the document is not valid in its format, or a text without a recognised format is not JSON | correct the document or read it with the right format |
| `E_NOT_FOUND` | the file does not exist | check the path |
| `E_IO` | the file cannot be read | see the message |
| `E_BINARY` | the content record holds binary data | query text documents only |
| `E_TYPE` | an input record is of another kind, or an entry is not a file | pass `content`, `entry`, `item` or `response` records |
| `E_MISSING_PARAM` | the source form has no `path` | add `path:` |
| `E_STALE` | the file changed while it was read | run the command again |
| `E_LIMIT` | the filter produces more results than one execution may hold, multiplies a string into a result longer than `limits.max_file_bytes_text` bytes (`"x" * n`), or the file is larger than the document size limit | narrow the filter, use `limit(n; f)`, or raise the limit |
| `E_CANCELLED` | the execution was cancelled or its time limit passed, including inside endless generators such as `repeat` | run the command again with a narrower filter |
| `E_TIMEOUT` | the execution was cancelled or its time limit passed, including inside endless generators such as `repeat` | run the command again with a narrower filter |

## Related

`read` with `key:` for values at key paths, `set` to change the values found
by `query`, `sql` for SQLite databases.
