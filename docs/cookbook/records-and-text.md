---
category: records-and-text
title: Records and text processing
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Records and text processing

## Sort records by a field

Sort the entries of a directory by size, largest first.

```nurt example
list "/home/user/app" |> sort size order: desc
```

```text expected
/home/user/app
```

## Take the first records

Take the ten most recently modified files.

```nurt example
find "/home/user/app" |> sort modified order: desc |> first 10
```

```text expected
/home/user/app
```

## Select fields

Show only the path and the size of the files.

```nurt example
find "/home/user/app" |> select path size
```

```text expected
path
```

## Compute a field

Show the size of the files in kilobytes.

```nurt example
find "/home/user/app" |> select path kb: size / 1000
```

```text expected
kb
```

## Group and summarize

Count the files by extension.

```nurt example
find "/home/user/app" |> group by: extension |> summarize
```

```text expected
extension
```

## Remove duplicates

List the distinct file extensions of a project.

```nurt example
find "/home/user/app" |> select extension |> distinct
```

```text expected
extension
```

## Skip a header

Skip the first line of a file.

```nurt example
lines "/home/user/data.csv" |> skip 1 |> first 5
```

```text expected
data.csv
```

## Split lines into fields

Split lines of a colon separated file into named fields.

```nurt example
lines "/etc/example/passwd" |> split ":" names: ["user", "uid", "home"]
```

```text expected
user
```

```bash
awk -F: '{print $1, $3}' /etc/example/passwd
```

## Parse lines with a pattern

Extract named parts of each log line.

```nurt example
lines "/home/user/app/app.log" |> parse /(?P<level>\w+) (?P<message>.*)/
```

```text expected
level
```

## Describe the fields of records

Show the fields and the types of the records of a command.

```nurt example
processes |> describe
```

```text expected
field
```

## Evaluate an expression

Evaluate an expression without reading any file.

```nurt example
evaluate 40 + 2
```

```text expected
42
```

## Name a result and use it twice

Store the result of a search and count it.

```nurt example
let recent = find "/home/user/app/src" glob: "**/*.ts" |> filter modified > ago(1h)
@recent |> count
```

```text expected
0
```

## Hash a file

Compute the SHA-256 hash of a file.

```nurt example
hash "/home/user/dl/a.tar.gz" algorithm: sha256
```

```text expected
sha256
```

```bash
sha256sum /home/user/dl/a.tar.gz
```

## Filter by a pattern in a field

Keep the entries whose name matches a regular expression.

```nurt example
list "/home/user/app" |> filter name ~ /^test_/
```

```text expected
test_
```
