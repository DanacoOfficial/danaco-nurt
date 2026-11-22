---
category: archives-and-network
title: Archives and network
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
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
