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

## Replace regardless of letter case

Replace a product name in every Markdown file whatever its letter case.

```nurt example
replace "acme" "Acme" in: ["/home/user/app/docs"] glob: "**/*.md" case_sensitive: false
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/docs/a.md","type":"file","content_utf8":"ACME and acme\n"}]}
```

```text expected
/home/user/app/docs/a.md
```

```bash
grep -rli 'acme' --include='*.md' /home/user/app/docs | xargs sed -i 's/acme/Acme/gI'
```

## Replace only inside comments

Change a spelling in comments and leave the code unchanged.

```nurt example
replace "colour" "color" in: ["/home/user/app/src"] glob: "**/*.go" scope: comments
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"// colour of the text\nvar colour = 1\n"}]}
```

```text expected
/home/user/app/src/a.go
```

## Replace across lines

Join a call that is split over two lines.

```nurt example
replace /open\(\n\s+path\)/ "open(path)" in: ["/home/user/app/src"] glob: "**/*.js" multiline: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.js","type":"file","content_utf8":"open(\n  path)\n"}]}
```

```text expected
/home/user/app/src/a.js
```

## Replace with a bound on the number of changes

Stop before any change when the pattern occurs fewer than two or more than five times.

```nurt example
replace "timeout = 30" "timeout = 60" in: ["/home/user/app/conf"] glob: "*.conf" expect: 2..5
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/conf/a.conf","type":"file","content_utf8":"timeout = 30\n"},{"path":"/home/user/app/conf/b.conf","type":"file","content_utf8":"timeout = 30\n"}]}
```

```text expected
/home/user/app/conf/a.conf
```

## Replace in files of one language

Replace a name in the files of one language, wherever they are.

```nurt example
replace "getUser" "fetchUser" in: ["/home/user/app"] file_type: ["ts"]
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.ts","type":"file","content_utf8":"getUser(1)\n"}]}
```

```text expected
/home/user/app/src/a.ts
```

## Replace and respect the ignore rules

Replace a name in the files that the repository does not ignore.

```nurt example
replace "v1/api" "v2/api" in: ["/home/user/app"] gitignore: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/client.py","type":"file","content_utf8":"url = 'v1/api'\n"}]}
```

```text expected
/home/user/app/client.py
```

## Remove lines by number

Remove the first two lines of a file, for example a banner.

```nurt example
lines "/home/user/app/export.csv" 1..2 |> remove
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/export.csv","type":"file","content_utf8":"banner\ngenerated\nid,name\n"}]}
```

```text expected
/home/user/app/export.csv
```

```bash
sed -i '1,2d' /home/user/app/export.csv
```

## Insert a header at the start of a file

Insert a shebang line at the start of a script.

```nurt example
insert "#!/bin/sh" in: "/home/user/app/run.sh" position: start
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/run.sh","type":"file","content_utf8":"echo run\n"}]}
```

```text expected
/home/user/app/run.sh
```

```bash
sed -i '1i #!/bin/sh' /home/user/app/run.sh
```

## Insert a line at the end of a file

Insert a closing line at the end of a file.

```nurt example
insert "# end of file" in: "/home/user/app/app.conf" position: end ensure_newline: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.conf","type":"file","content_utf8":"a = 1"}]}
```

```text expected
/home/user/app/app.conf
```

## Copy the non-empty lines to another file

Write the non-empty lines of a file to a new file.

```nurt example
lines "/home/user/app/in.txt" |> filter text != "" |> write "/home/user/app/out.txt" mode: create
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/in.txt","type":"file","content_utf8":"a\n\nb\n"}]}
```

```text expected
/home/user/app/out.txt
```

```bash
grep -v '^$' /home/user/app/in.txt > /home/user/app/out.txt
```

## Write several files in one transaction

Create two files together; when one of them cannot be created, neither is.

```nurt example
write files: [{path: "/home/user/app/a.txt", text: "one\n"}, {path: "/home/user/app/b.txt", text: "two\n"}] mode: create
```

```text expected
/home/user/app/a.txt
```

## Convert all text files to LF

Convert the line endings of every text file of a directory.

```nurt example
find "/home/user/app/docs" glob: "**/*.txt" |> convert line_ending: lf
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/docs/a.txt","type":"file","content_utf8":"a\r\n"}]}
```

```text expected
/home/user/app/docs/a.txt
```

## Add a byte order mark

Add a byte order mark to a file that a legacy program reads.

```nurt example
convert "/home/user/legacy/list.csv" bom: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/legacy/list.csv","type":"file","content_utf8":"a;b\n"}]}
```

```text expected
/home/user/legacy/list.csv
```

## Revert a patch

Apply a unified diff in reverse to undo the change that it describes.

```nurt example
patch "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+two\n" root: "/home/user/app" reverse: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/a.txt","type":"file","content_utf8":"two\n"}]}
```

```text expected
/home/user/app/a.txt
```

```bash
patch -R -p1 -d /home/user/app < change.diff
```

## Apply a patch from a file

Apply a diff stored in a file and strip one leading path component.

```nurt example
patch from: "/home/user/dl/change.diff" root: "/home/user/app" strip: 1
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/dl/change.diff","type":"file","content_utf8":"--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+two\n"},{"path":"/home/user/app/a.txt","type":"file","content_utf8":"one\n"}]}
```

```text expected
/home/user/app/a.txt
```
