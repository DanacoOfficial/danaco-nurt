---
category: search-and-read
title: Searching and reading
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Searching and reading

## List the entries of a directory

List the files and directories directly under a project directory.

```nurt example
list "/home/user/app"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n"},{"path":"/home/user/app/docs","type":"dir"}]}
```

```text expected
/home/user/app/main.go
```

```bash
ls -la /home/user/app
```

```powershell
Get-ChildItem C:\Users\user\app
```

## Find the largest Go files changed in the last day

Find Go files larger than 1 MB that changed in the last 24 hours, largest first.

```nurt example
find "/home/user/app" glob: "**/*.go"
|> filter size > 1mb and modified > ago(24h)
|> sort size order: desc
|> first 20
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/gen/schema.go","type":"file","content_utf8":"package gen\n"}]}
```

```text expected
matched 0
```

```bash
find /home/user/app -name '*.go' -size +1M -mtime -1 -printf '%s %p\n' | sort -rn | head -20
```

```powershell
Get-ChildItem C:\Users\user\app -Recurse -Filter *.go | Where-Object { $_.Length -gt 1MB -and $_.LastWriteTime -gt (Get-Date).AddDays(-1) } | Sort-Object Length -Descending | Select-Object -First 20
```

## Count the files of one type

Count the TypeScript files of a project.

```nurt example
find "/home/user/app" glob: "**/*.ts" |> count
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/a.ts","type":"file","content_utf8":""},{"path":"/home/user/app/b.ts","type":"file","content_utf8":""}]}
```

```text expected
2
```

```bash
find /home/user/app -name '*.ts' | wc -l
```

## Find files by a name fragment

Find files whose name contains the word `config`, in any directory.

```nurt example
find "/home/user/app" glob: "**/*config*"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/config.toml","type":"file","content_utf8":""},{"path":"/home/user/app/src/main.rs","type":"file","content_utf8":""}]}
```

```text expected
/home/user/app/config.toml
```

```bash
find /home/user/app -name '*config*'
```

## Find a text pattern in source files

Find every call of a function in the Go files of a directory.

```nurt example
search /oldName\(/ "/home/user/app/src" glob: "**/*.go"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"x := oldName(1)\n"}]}
```

```text expected
/home/user/app/src/a.go
```

```bash
grep -rn 'oldName(' --include='*.go' /home/user/app/src
```

```powershell
Get-ChildItem C:\Users\user\app\src -Recurse -Filter *.go | Select-String 'oldName\('
```

## List the files that contain a pattern

List only the paths of the files that contain a pattern, without the matching lines.

```nurt example
search /TODO/ "/home/user/app/src" output: files
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"// TODO\n"},{"path":"/home/user/app/src/b.go","type":"file","content_utf8":"x\n"}]}
```

```text expected
/home/user/app/src/a.go
```

```bash
grep -rl 'TODO' /home/user/app/src
```

## Search without regard to letter case

Search for a word in a log file and accept any letter case.

```nurt example
search "error" "/home/user/app/app.log" case_sensitive: false
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.log","type":"file","content_utf8":"ok\nERROR disk full\n"}]}
```

```text expected
disk full
```

```bash
grep -in 'error' /home/user/app/app.log
```

## Show context around a match

Show three lines before and after each match of a pattern.

```nurt example
search /panic/ "/home/user/app/app.log" context: 3
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.log","type":"file","content_utf8":"a\nb\npanic: boom\nc\n"}]}
```

```text expected
panic: boom
```

```bash
grep -C 3 'panic' /home/user/app/app.log
```

## Read a file

Read the whole content of a text file.

```nurt example
read "/home/user/app/README.md"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/README.md","type":"file","content_utf8":"# App\n"}]}
```

```text expected
# App
```

```bash
cat /home/user/app/README.md
```

```powershell
Get-Content C:\Users\user\app\README.md
```

## Read a range of lines

Read lines 10 to 20 of a source file with line numbers.

```nurt example
read "/home/user/app/main.go" lines: 10..20 line_numbers: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n"}]}
```

```text expected
main.go
```

```bash
sed -n '10,20p' /home/user/app/main.go
```

## Read the last lines of a log

Read the last 50 lines of a log file.

```nurt example
read "/home/user/app/app.log" last: 50
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.log","type":"file","content_utf8":"started\n"}]}
```

```text expected
started
```

```bash
tail -n 50 /home/user/app/app.log
```

```powershell
Get-Content C:\Users\user\app\app.log -Tail 50
```

## Read the lines around a pattern

Read the lines around the first line that matches a pattern.

```nurt example
read "/home/user/app/main.go" around: /func main/ context: 3
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n\nfunc main() {}\n"}]}
```

```text expected
func main
```

## Show the metadata of a file

Show the size, times and permissions of a file together with its SHA-256 hash.

```nurt example
info "/home/user/app/main.go" hash: sha256
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n"}]}
```

```text expected
/home/user/app/main.go
```

```bash
stat /home/user/app/main.go && sha256sum /home/user/app/main.go
```

## Count the lines of a file

Count the lines of a file.

```nurt example
lines "/home/user/app/main.go" |> count
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"a\nb\nc\n"}]}
```

```text expected
3
```

```bash
wc -l /home/user/app/main.go
```
