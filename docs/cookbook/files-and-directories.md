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

## List the largest entries

List the entries of a directory, the largest first.

```nurt example
list "/home/user/app" sort: size order: desc
```

```text expected
/home/user/app
```

```bash
ls -lS /home/user/app
```

## Show a directory tree

Show the tree of a directory down to the third level.

```nurt example
list "/home/user/app" view: tree depth: 3
```

```text expected
/home/user/app
```

```bash
tree -L 3 /home/user/app
```

## List only the subdirectories

List the subdirectories of a directory without hidden entries.

```nurt example
list "/home/user/app" type: directory hidden: false
```

```text expected
/home/user/app
```

## Find the largest files

Find the ten largest files of a directory.

```nurt example
find "/home/user/data" |> filter size > 100mb |> sort size order: desc |> first 10
```

```text expected
nothing_matched
```

```bash
find /home/user/data -type f -size +100M -exec ls -lS {} + | head -10
```

## Find the files changed today

Find the files that changed during the last 24 hours.

```nurt example
find "/home/user/app/src" |> filter modified > ago(1d)
```

```text expected
nothing_matched
```

```bash
find /home/user/app/src -type f -mtime -1
```

## Find a file by an approximate name

Find the files whose path resembles a name.

```nurt example
find "/home/user/app" fuzzy: "readme" exclude_preset: ["vcs", "deps"]
```

```text expected
README
```

## Show the hash of a file

Show the BLAKE3 hash of a file together with its size.

```nurt example
hash "/home/user/dl/image.iso" algorithm: blake3
```

```text expected
blake3
```

```bash
b3sum /home/user/dl/image.iso
```

## Copy without overwriting

Copy a directory and keep the files that already exist in the target.

```nurt example
copy "/home/user/photos" to: "/home/user/backup/photos" on_conflict: skip
```

```text expected
/home/user/backup/photos
```

```bash
rsync -a --ignore-existing /home/user/photos/ /home/user/backup/photos/
```

## Move a file

Move a file to another path.

```nurt example
move "/home/user/dl/report.pdf" to: "/home/user/docs/report.pdf"
```

```text expected
/home/user/docs/report.pdf
```

```bash
mv /home/user/dl/report.pdf /home/user/docs/report.pdf
```

## Update the modification time

Create a file when it is missing and update its modification time otherwise.

```nurt example
create "/home/user/app/.stamp" type: file on_exists: touch
```

```text expected
/home/user/app/.stamp
```

```bash
touch /home/user/app/.stamp
```

## Create several directories

Create the directories of a project layout in one call.

```nurt example
create ["/home/user/app/src", "/home/user/app/docs", "/home/user/app/tests"] type: directory
```

```text expected
/home/user/app/src
```

```bash
mkdir -p /home/user/app/src /home/user/app/docs /home/user/app/tests
```

## Create a temporary directory

Create a directory that Nurt removes at the end of the session.

```nurt example
create "/home/user/tmp/work" type: directory temporary: true
```

```text expected
/home/user/tmp/work
```

## Change the account and group of a tree

Set the account and group of a directory tree.

```nurt example
attributes "/home/user/www" owner: "deploy" group: "deploy" recursive: true
```

```text expected
/home/user/www
```

```bash
chown -R deploy:deploy /home/user/www
```

## Make a file read-only

Set the read-only attribute of a file.

```nurt example
attributes "/home/user/app/config.lock" readonly: true
```

```text expected
/home/user/app/config.lock
```

```bash
chmod a-w /home/user/app/config.lock
```

## Fail when a file is missing

Remove a file and report an error when it does not exist.

```nurt example
remove "/home/user/app/app.pid" on_missing: error
```

```text expected
E_NOT_FOUND
```

## Compare files ignoring whitespace

Show the differences between two files and ignore changes of whitespace.

```nurt example
diff "/home/user/a.txt" "/home/user/b.txt" ignore_whitespace: true context: 1
```

```text expected
hunk
```

```bash
diff -w -U 1 /home/user/a.txt /home/user/b.txt
```
