---
category: archives-and-network
title: Archives and network
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/archives-and-network.md -->
<!-- Responsibility:   Documents Archives and network. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Archives and network

## Extract an archive

Extract a compressed tar archive into a directory.

```nurt example
archive.extract "/home/user/dl/data.tar.gz" into: "/home/user/data"
```

```text expected
/home/user/data
```

```bash
tar -xzf /home/user/dl/data.tar.gz -C /home/user/data
```

## List the content of an archive

List the entries of an archive without extracting it.

```nurt example
archive.list "/home/user/dl/data.zip"
```

```text expected
data
```

## Extract only some entries

Extract the entries that match a pattern.

```nurt example
archive.extract "/home/user/dl/data.zip" into: "/home/user/data" only: "**/*.csv"
```

```text expected
/home/user/data
```

## Create an archive

Create a ZIP archive of a directory.

```nurt example
find "/home/user/app/dist" |> archive.create "/home/user/out/dist.zip" base: "/home/user/app"
```

```text expected
/home/user/out/dist.zip
```

```bash
cd /home/user/app && zip -r /home/user/out/dist.zip dist
```

## Compress a file

Compress a file with gzip and keep the original.

```nurt example
compress "/home/user/app/app.log"
```

```text expected
app.log
```

## Decompress a file

Decompress a gzip file.

```nurt example
decompress "/home/user/app/app.log.gz"
```

```text expected
app.log
```

## Send an HTTP request

Send a GET request and require one response.

```nurt example
http.request "https://example.com/health" |> assert count: 1
```

```text expected
200
```

```bash
curl -fsS https://example.com/health
```

## Post JSON

Send a JSON body with a POST request.

```nurt example
http.request "https://example.com/api/items" method: post json: {"name": "item"}
```

```text expected
200
```

## Send a request with a header

Send a request with an authorization header.

```nurt example
http.request "https://example.com/api/me" headers: {"Accept": "application/json"}
```

```text expected
200
```

## Read a page as Markdown

Read the main text of a web page as Markdown.

```nurt example
http.request "https://example.com/docs" as: markdown
```

```text expected
example
```

## Download a file and verify its hash

Download a file and verify its SHA-256 hash.

```nurt example
http.download "https://example.com/a.tar.gz" "/home/user/dl/a.tar.gz" hash: "sha256:9d3c"
```

```text expected
/home/user/dl/a.tar.gz
```

```bash
curl -fLo a.tar.gz https://example.com/a.tar.gz && sha256sum a.tar.gz
```

## Check that a host answers

Send four echo requests to a host.

```nurt example
network.ping "example.com" count: 4
```

```text expected
example.com
```

## Resolve a name

Resolve the IPv6 addresses of a name.

```nurt example
network.resolve "example.com" type: aaaa
```

```text expected
example.com
```

## List the network interfaces

List the network interfaces of the host.

```nurt example
network.interfaces
```

```text expected
interface
```

## Extract an archive without its top directory

Extract a source archive and drop the first path component of every entry.

```nurt example
archive.extract "/home/user/dl/src.tar.gz" into: "/home/user/src" strip_components: 1
```

```text expected
/home/user/src
```

```bash
tar -xzf /home/user/dl/src.tar.gz -C /home/user/src --strip-components=1
```

## Extract without overwriting files

Extract an archive and keep the files that already exist.

```nurt example
archive.extract "/home/user/dl/data.zip" into: "/home/user/data" on_conflict: skip
```

```text expected
/home/user/data
```

## List the entries that match a pattern

List only the JSON entries of an archive.

```nurt example
archive.list "/home/user/dl/data.zip" glob: "**/*.json"
```

```text expected
json
```

```bash
unzip -l /home/user/dl/data.zip '*.json'
```

## Create a Zstandard tar archive

Pack a source directory into a tar archive compressed with Zstandard.

```nurt example
find "/home/user/app/src" |> archive.create "/home/user/out/src.tar.zst" format: tar_zst base: "/home/user/app"
```

```text expected
/home/user/out/src.tar.zst
```

```bash
tar --zstd -cf /home/user/out/src.tar.zst -C /home/user/app src
```

## Compress a file with a chosen algorithm

Compress a log with Zstandard at level 9 and remove the original.

```nurt example
compress "/home/user/app/app.log" algorithm: zstd level: 9 keep: false
```

```text expected
app.log
```

```bash
zstd -9 --rm /home/user/app/app.log
```

## Compress the old logs

Compress the log files that are older than seven days.

```nurt example
find "/home/user/logs" glob: "*.log" depth: 1 |> filter modified < ago(7d) |> compress
```

```text expected
nothing_matched
```

## Request only the headers

Send a HEAD request to read the headers of a resource.

```nurt example
http.request "https://example.com/a.tar.gz" method: head
```

```text expected
200
```

```bash
curl -fsSI https://example.com/a.tar.gz
```

## Save a response to a file

Save the body of a response to a file.

```nurt example
http.request "https://example.com/data.json" save: "/home/user/dl/data.json"
```

```text expected
/home/user/dl/data.json
```

## List the links of a page

List the links of a page that lead to the same host.

```nurt example
http.request "https://example.com/docs" extract: links same_host: true
```

```text expected
example.com
```

## Limit the time and retry a request

Fail a request after five seconds and retry it up to three times.

```nurt example
http.request "https://example.com/health" timeout: 5s retries: 3
```

```text expected
200
```

```bash
curl -fsS --max-time 5 --retry 3 https://example.com/health
```

## Resolve a mail exchanger

Resolve the mail exchangers of a domain through a chosen DNS server.

```nurt example
network.resolve "example.com" type: mx server: "9.9.9.9"
```

```text expected
example.com
```

```bash
dig @9.9.9.9 example.com MX
```

## Ping with a short timeout

Send one echo request and wait at most one second.

```nurt example
network.ping "example.com" count: 1 timeout: 1s
```

```text expected
example.com
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
