---
category: edit-many-files
title: Editing many files
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Editing many files

## Replace an identifier in all Go files

Replace a function name in every Go file under a directory.

```nurt example
replace /oldName\(/ "newName(" in: ["/home/user/app/src"] glob: "**/*.go"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"x := oldName(1)\n"}]}
```

```text expected
/home/user/app/src/a.go
```

```bash
grep -rl 'oldName(' --include='*.go' /home/user/app/src | xargs sed -i 's/oldName(/newName(/g'
```

## Replace only when the expected number of files matches

Stop before any change when the pattern occurs in a different number of files than expected.

```nurt example
search /oldName\(/ "/home/user/app/src" glob: "**/*.go" |> assert paths: 1 |> replace "newName("
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"x := oldName(1)\n"}]}
```

```text expected
/home/user/app/src/a.go
```

## Replace a literal text

Replace a literal text that contains characters special to regular expressions.

```nurt example
replace "a.b(c)" "x" in: ["/home/user/app/notes.txt"] mode: literal
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/notes.txt","type":"file","content_utf8":"call a.b(c) now\n"}]}
```

```text expected
/home/user/app/notes.txt
```

## Replace only the first occurrence in each file

Replace the first occurrence of a text in each file and leave the others.

```nurt example
replace "http://" "https://" in: ["/home/user/app/site"] glob: "**/*.html" occurrence: 1
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/site/index.html","type":"file","content_utf8":"http://a http://b\n"}]}
```

```text expected
/home/user/app/site/index.html
```

## Replace a word without touching longer identifiers

Replace a whole word only, so that longer identifiers that contain it stay unchanged.

```nurt example
replace "id" "uid" in: ["/home/user/app/src"] glob: "**/*.py" whole_word: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.py","type":"file","content_utf8":"id = 1\nvalid = 2\n"}]}
```

```text expected
/home/user/app/src/a.py
```

## Replace and verify with a build

Replace a name and run the build in the same program; the changes are rolled back when the build fails.

```nurt example
search /oldName\(/ "/home/user/app/src" glob: "**/*.go"
|> replace "newName("
|> verify (run ["go", "build", "./..."] cwd: "/home/user/app" |> assert exit_code: 0)
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"x := oldName(1)\n"}]}
```

```text expected
/home/user/app/src/a.go
```

## Preview a replacement

Compute the targets of a replacement without changing anything.

```nurt example
replace /oldName\(/ "newName(" in: ["/home/user/app/src"] glob: "**/*.go"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"x := oldName(1)\n"}]}
```

```text expected
/home/user/app/src/a.go
```

The call parameter `preview: true` of the request turns the program into a preview.

## Remove the matching lines of many files

Remove the lines that contain a debug statement from a file.

```nurt example
lines "/home/user/app/app.js" |> filter text ~ /console\.log/ |> remove
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.js","type":"file","content_utf8":"a();\nconsole.log(1);\n"}]}
```

```text expected
/home/user/app/app.js
```

```bash
sed -i '/console\.log/d' /home/user/app/app.js
```

## Insert a line after a matching line

Insert an import after the package clause of a Go file.

```nurt example
lines "/home/user/app/main.go" |> filter text == "package main" |> insert r#"import "fmt""# position: after
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n"}]}
```

```text expected
/home/user/app/main.go
```

```bash
sed -i '/^package main$/a import "fmt"' /home/user/app/main.go
```

## Append a line to a file

Append one line to a file that exists.

```nurt example
write "/home/user/app/.gitignore" "node_modules/\n" mode: append
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/.gitignore","type":"file","content_utf8":"dist/\n"}]}
```

```text expected
/home/user/app/.gitignore
```

```bash
echo "node_modules/" >> /home/user/app/.gitignore
```

## Sort a file in place

Sort the lines of a file and write the result to the same file.

```nurt example
lines "/home/user/app/names.txt" |> sort text |> write "/home/user/app/names.txt" mode: overwrite
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/names.txt","type":"file","content_utf8":"b\na\n"}]}
```

```text expected
/home/user/app/names.txt
```

## Change the line endings of many files

Convert text files to LF line endings.

```nurt example
convert "/home/user/app/a.txt" line_ending: lf
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/a.txt","type":"file","content_utf8":"a\r\nb\r\n"}]}
```

```text expected
/home/user/app/a.txt
```

```bash
dos2unix /home/user/app/a.txt
```

## Convert the encoding of a file

Convert a file from Windows-1250 to UTF-8.

```nurt example
convert "/home/user/legacy/plik.txt" encoding: "utf-8"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/legacy/plik.txt","type":"file","content_utf8":"text\n"}]}
```

```text expected
/home/user/legacy/plik.txt
```

```bash
iconv -f windows-1250 -t utf-8 /home/user/legacy/plik.txt
```

## Apply a patch

Apply a unified diff to a project directory.

```nurt example
patch "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+two\n" root: "/home/user/app"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/a.txt","type":"file","content_utf8":"one\n"}]}
```

```text expected
/home/user/app/a.txt
```

```bash
patch -p1 -d /home/user/app < change.diff
```
