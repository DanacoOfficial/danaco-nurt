---
category: files-and-directories
title: Files and directories
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Files and directories

## Remove a directory

Remove a directory with its content.

```nurt example
remove "/home/user/app/build"
```

```text expected
/home/user/app/build
```

```bash
rm -rf /home/user/app/build
```

## Remove the files of a pattern

Remove the temporary files of a build directory.

```nurt example
find "/home/user/app/build" glob: "**/*.tmp" |> remove
```

```text expected
nothing_matched
```

## Remove the content of a directory

Remove everything inside a directory and keep the directory.

```nurt example
find "/home/user/app/build" glob: "*" depth: 1 |> remove
```

```text expected
nothing_matched
```

## Remove a file with a space in its name

Remove a file whose name contains a space; the path is one value.

```nurt example
remove "/home/user/data/my file.txt"
```

```text expected
my file.txt
```

## Copy a directory

Copy a directory and keep the attributes.

```nurt example
copy "/home/user/app" to: "/home/user/backup" preserve: true
```

```text expected
/home/user/backup
```

```bash
cp -a /home/user/app /home/user/backup
```

## Copy several files into a directory

Copy two files into a directory.

```nurt example
copy ["/home/user/a.txt", "/home/user/b.txt"] into: "/home/user/out"
```

```text expected
/home/user/out
```

## Move the old logs

Move the log files older than seven days to an archive directory.

```nurt example
find "/home/user/logs" glob: "*.log" depth: 1 |> filter modified < ago(7d) |> move into: "/home/user/logs/old"
```

```text expected
nothing_matched
```

```bash
find /home/user/logs -maxdepth 1 -name '*.log' -mtime +7 -exec mv {} /home/user/logs/old/ \;
```

## Rename a file

Rename a file inside its directory.

```nurt example
rename path: "/home/user/a.txt" to: "b.txt"
```

```text expected
b.txt
```

## Rename files by a pattern

Replace the extension of the files of a directory.

```nurt example
find "/home/user/photos" glob: "*.jpeg" depth: 1 |> rename /\.jpeg$/ ".jpg"
```

```text expected
nothing_matched
```

## Create a directory with its parents

Create a nested directory.

```nurt example
create "/home/user/app/out/reports" type: directory
```

```text expected
/home/user/app/out/reports
```

## Create a symbolic link

Create a link to a directory.

```nurt example
create "/home/user/current" type: link target: "/home/user/app/releases/42"
```

```text expected
/home/user/current
```

## Change permissions

Make a script executable.

```nurt example
attributes "/home/user/app/run.sh" mode: "755"
```

```text expected
/home/user/app/run.sh
```

## Show the size of the subdirectories

List the directory tree with the size of every directory.

```nurt example
list "/home/user/app" view: sizes depth: 2
```

```text expected
/home/user/app
```

```bash
du -h --max-depth=2 /home/user/app
```

## Find duplicates by hash

Compute the hash of every file of a directory and keep the groups that share a hash.

```nurt example
find "/home/user/photos" |> hash |> group by: sha256
```

```text expected
sha256
```

## Compare two files

Show the differences between two files.

```nurt example
diff "/home/user/a.txt" "/home/user/b.txt"
```

```text expected
hunk
```
