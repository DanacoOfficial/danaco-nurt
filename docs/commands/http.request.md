---
command: http.request
group: network
summary: Send an HTTP request and return the response, optionally as text or Markdown.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# http.request

## Synopsis

```text
http.request <url> [name: value]...
```

## Description

`http.request` sends one request for each address of `url` and returns one
`response` record per address, in the order of the addresses. A list of
addresses is fetched with up to four requests at the same time. With
`extract: tables` the command returns `row` records instead, one per row of
each selected table.

An address without a scheme is requested with `https://`, and the report
carries the note `scheme_assumed`. Host names are converted to Punycode, and
IPv4 addresses written in decimal, octal or hexadecimal form are converted to
dotted form before the request. Only the schemes `http` and `https` are
accepted.

A status of 400 or above is a result, not an error: the status is the field
`status` of the record. Redirects with the statuses 301, 302, 303, 307 and 308
are followed up to `max_redirects`; the addresses passed are listed in
`redirects` and the last one is `final_url`. A redirect to another host drops
the headers `Authorization`, `Cookie` and `Proxy-Authorization`. A redirect
from `https` to `http` is not followed: the request fails with `E_NETWORK` and
the problem `scheme_downgrade`, and `follow_redirects: false` returns the
redirect itself. A `303` turns a request into a `get` unless it is a `head`,
which is sent again as a `head`. With
`retries`, a request that fails to connect, is reset, or receives the status
429 or a status of 500 and above is sent again after a pause; `Retry-After`
sets the pause when the server gives one. Requests with the methods `post` and
`patch` are not sent again.

Bodies of the types `gzip` and `deflate` are decompressed. The character set
of a text body is taken from, in order: `charset`, a byte order mark, the
`charset` parameter of `Content-Type`, a `<meta>` declaration of an HTML page
and detection from the bytes. A body that is not text, such as an image, can
only be written to a file with `save`.

With `as: text` or `as: markdown` an HTML page is converted to plain text or
Markdown. `main_only` keeps the main content and drops navigation, headers,
footers and forms; `links` and `images` select how links and images are
written. Markdown and plain text documents are returned in the same forms.
Parts of a page are selected with `section` (the content under one heading),
`find` (the paragraphs that match a pattern, with `find_context` paragraphs
around each) and `query` (the `query_limit` paragraphs most relevant to the
query, in document order). With `outline: auto`, a page longer than the
`max_chars` option of the call is returned as an outline: the title, the
length, the sections with their sizes, the start of the content and the path
of a file that holds the full content. `offset` skips characters of the
result; `total_chars` gives the length of the whole result.

The response cache keeps successful `get` responses of requests without own
headers. `cache: use` answers from a fresh cached response, `revalidate`
asks the server whether a cached response is still current with
`If-None-Match` and `If-Modified-Since`, and `only` answers from the cache
without any network access. `unchanged_since` takes the `etag` of an earlier
result; when the body is the same, the record has `unchanged: true` and an
empty body.

The console applies no network rules of its own. Every address, including
loopback and private addresses, is contacted. The check of a program
(`nurt_check`) lists every request in the `network` entry of its details with
the normalized address, host, port, scheme, proxy, header names, body kind,
cache mode and, for IP addresses, the address class. An embedding application
that controls network access can refuse connections before they are opened.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `url` | `text \| list<text>` | required | Address, or several addresses with one result each. |
| `method` | `enum(get\|head\|post\|put\|patch\|delete\|options)` | `get` | HTTP method. |
| `headers` | `map<text>` | none | Request headers. |
| `body` | `text \| path` | none | Request body as text or from a file. |
| `json` | `any` | none | Value sent as a JSON body with the matching Content-Type header. |
| `save` | `path` | none | Write the response body to this file; the body of an answer with a status outside 200 to 299 is not written and the record has no `saved` field. |
| `as` | `enum(raw\|text\|markdown)` | `raw` | Return an HTML page unchanged, as plain text or as Markdown with an outline. |
| `extract` | `enum(content\|headers\|links\|headings\|meta\|tables)` | `content` | Part of the page returned: content, headers, links, headings, metadata or tables. |
| `table` | `int` | `0` | Number of the table returned with extract set to tables, counted from 1; 0 returns all tables. |
| `main_only` | `bool` | `true` | Keep only the main content of the page when converting to text or Markdown. |
| `links` | `enum(inline\|list\|none)` | `inline` | Keep links inline, collect them in a list, or drop them when converting. |
| `images` | `enum(drop\|alt)` | `alt` | Drop images or keep their alternative text when converting. |
| `section` | `text` | none | Return only the section under the heading with this text. |
| `outline` | `enum(auto\|always\|never)` | `auto` | Return an outline of a long page before its content. |
| `query` | `text` | none | Return only the paragraphs of the page most relevant to this query. |
| `query_limit` | `int` | `5` | Maximum number of paragraphs returned for query, from 1 to 20. |
| `find` | `regex` | none | Return only the paragraphs that match this pattern. |
| `find_context` | `int` | `1` | Number of paragraphs of context around each paragraph matched by find, up to 10. |
| `links_filter` | `regex` | none | Only links whose address matches this pattern, with extract set to links. |
| `same_host` | `bool` | `false` | Only links to the same host, with extract set to links. |
| `charset` | `text` | `"auto"` | Character set used to decode the body. |
| `offset` | `int` | `0` | Number of characters of the body to skip. |
| `cache` | `enum(bypass\|use\|revalidate\|only)` | `bypass` | Ignore the response cache, use it, revalidate cached responses, or answer only from the cache without the network. |
| `unchanged_since` | `text` | none | Etag of a known version; an unchanged response returns no body. |
| `timeout` | `duration` | `30s` | Time limit of the request. |
| `follow_redirects` | `bool` | `true` | Follow redirects. |
| `max_redirects` | `int` | `10` | Maximum number of redirects followed. |
| `verify_tls` | `bool` | `true` | Verify the TLS certificate of the server. |
| `retries` | `int` | `0` | Number of retries after a connection failure. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `response` |

The `response` record has the fields `url`, `status`, `headers` (names in
lower case; repeated headers joined with `, `, `set-cookie` as a list),
`body`, `bytes`, `elapsed`, `final_url`, `saved`, `content_type`, `charset`,
`truncated`, `offset`, `total_chars`, `redirects`, `from_cache`, `unchanged`,
`etag`, `last_modified` and `remote_address`. `etag` is the first 12
hexadecimal digits of the SHA-256 of the returned body. A `row` record holds
the cells of one table row by column name, the row number `no` and the table
number `table`.

The text form of the result shows the columns `url`, `status`, `bytes`,
`content_type` and `elapsed`, and never the body. The body and the other
fields stay in the JSON form and appear in the text with verbosity `detailed`
or when the request names them in `fields`, for example `fields: ["status",
"body"]`.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |
| Reversibility conditions | Methods get, head and options only read; other methods change the remote side and are irreversible; a file written by save is reversible. |

| Action | Effect | Reversibility | Note |
|---|---|---|---|
| `post` | `external` | `irreversible` | Applies when method is post. |
| `put` | `external` | `irreversible` | Applies when method is put. |
| `patch` | `external` | `irreversible` | Applies when method is patch. |
| `delete` | `external` | `irreversible` | Applies when method is delete. |
| `save` | `local` | `reversible` | Applies to the file written when save is given. |

## Examples

Fetch a documentation page as Markdown:

```nurt example
http.request "https://docs.example.com/install" as: markdown
```

```text expected
# Installation
```

Return the three paragraphs of a page most relevant to a question:

```nurt example
http.request "https://docs.example.com/guide" as: text query: "proxy configuration" query_limit: 3
```

Save a release archive to a file:

```nurt example
http.request "https://example.com/releases/app.tar.gz" save: "/srv/downloads/app.tar.gz"
```

Read one section of a page from the cache filled by an earlier request:

```nurt example
http.request "https://docs.example.com/guide" as: markdown section: "Upgrade" cache: use
```

Send a JSON body and require a successful status:

```nurt example
http.request "https://api.example.com/items" method: post json: {name: "demo"} |> assert all: status < 400
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_ARG` | An address is not valid, parameters conflict (`problem: param_conflict`), a count is out of range, `offset` is past the end of the result, or `save` or a changing method is used where only reading is allowed. | Correct the address or remove the conflicting parameter. |
| `E_UNKNOWN_VALUE` | A parameter has a value outside its enumeration. | Use one of the listed values. |
| `E_TYPE` | A parameter has a value of another type. | Give a value of the type in the table. |
| `E_REGEX` | The pattern of `find` or `links_filter` is not valid. | Correct the pattern. |
| `E_NETWORK` | The request failed. `problem` tells why: `dns`, `connect`, `reset`, `tls`, `proxy`, `redirect_limit`, `scheme_downgrade`, `bad_location`, `protocol`, `decode` or `denied`. | Check the address, the proxy and the certificate, or raise `max_redirects`. |
| `E_TIMEOUT` | The request did not finish within `timeout`. | Raise `timeout`. |
| `E_CANCELLED` | The program was cancelled during the request. | Run the program again. |
| `E_NOT_FOUND` | `cache: only` found no cached response, the page has no section `section`, or the page has fewer tables than `table`. | Fetch the page with `cache: use` first, or use a section or table from `expected`. |
| `E_BINARY` | The body is not text. | Write the body to a file with `save`. |
| `E_CONFIG` | The file of `http.ca_file` cannot be read. | Correct the setting. |

## Related

`http.download`, `network.resolve`, `undo`, `help group: "network"`.
