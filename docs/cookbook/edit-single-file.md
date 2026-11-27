---
category: edit-single-file
title: Editing a single file
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/edit-single-file.md -->
<!-- Responsibility:   Documents Editing a single file. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Editing a single file

## Create a file

Create a new file with a given content; the program fails when the file exists.

```nurt example
write "/home/user/app/notes.txt" "first\n" mode: create
```

```text expected
/home/user/app/notes.txt
```

## Overwrite a file

Replace the whole content of an existing file.

```nurt example
write "/home/user/app/notes.txt" "second\n" mode: overwrite
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/notes.txt","type":"file","content_utf8":"first\n"}]}
```

```text expected
/home/user/app/notes.txt
```

## Create a file with several lines

Create a multi-line file without escaping quotation marks.

```nurt example
write "/home/user/app/Dockerfile" r#"FROM alpine:3.20
RUN apk add --no-cache curl
CMD ["sh"]
"# mode: create
```

```text expected
/home/user/app/Dockerfile
```

```bash
cat > /home/user/app/Dockerfile <<'EOF'
FROM alpine:3.20
RUN apk add --no-cache curl
CMD ["sh"]
EOF
```

## Prepend a line to a file

Add a header line at the start of a file.

```nurt example
write "/home/user/app/notes.txt" "# Notes\n" mode: prepend
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/notes.txt","type":"file","content_utf8":"a\n"}]}
```

```text expected
/home/user/app/notes.txt
```

## Insert a line at a line number

Insert a comment before line 3 of a file.

```nurt example
insert "// updated" in: "/home/user/app/main.go" line: 3 position: before
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"a\nb\nc\n"}]}
```

```text expected
/home/user/app/main.go
```

## Apply several changes as one transaction

Apply three changes to one file; when one of them does not apply, none is applied.

```nurt example
edit "/home/user/app/main.go" changes: [
  {replace: "oldName(", with: "newName(", occurrence: 1},
  {insert: "// updated", line: 3, position: before},
  {delete_lines: 10..12}
] expect: 3
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"oldName(1)\n"}]}
```

```text expected
/home/user/app/main.go
```

## Set a key in a JSON file

Set one key of a JSON file and keep the layout of the rest.

```nurt example
set "/home/user/app/package.json" key: "scripts.test" value: "vitest"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/package.json","type":"file","content_utf8":"{\n  \"scripts\": {\n    \"test\": \"jest\"\n  }\n}\n"}]}
```

```text expected
/home/user/app/package.json
```

```bash
jq '.scripts.test = "vitest"' package.json > p.tmp && mv p.tmp package.json
```

## Set a key in a TOML file

Change a version in a TOML manifest and keep its comments.

```nurt example
set "/home/user/app/Cargo.toml" key: "package.version" value: "0.4.0"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/Cargo.toml","type":"file","content_utf8":"[package]\nversion = \"0.3.9\" # bump on release\n"}]}
```

```text expected
/home/user/app/Cargo.toml
```

## Set several keys at once

Set several keys of a YAML file in one operation.

```nurt example
set "/home/user/app/config.yaml" values: {"server.port": 8080, "server.debug": false}
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/config.yaml","type":"file","content_utf8":"server:\n  port: 80\n"}]}
```

```text expected
/home/user/app/config.yaml
```

## Read a key of a structured document

Read one value from a JSON document.

```nurt example
read "/home/user/app/package.json" key: "scripts.test"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/package.json","type":"file","content_utf8":"{\"scripts\":{\"test\":\"jest\"}}"}]}
```

```text expected
jest
```

## Query a document

Select the names of all dependencies of a JSON manifest.

```nurt example
read "/home/user/app/package.json" |> query "dependencies"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/package.json","type":"file","content_utf8":"{\"dependencies\":{\"left-pad\":\"1.0.0\"}}"}]}
```

```text expected
left-pad
```

## Run a query on a SQLite database

Read the active users of a SQLite database.

```nurt example
sql "/home/user/app/app.db" "select id, name from users where active = ? limit 5" params: [1]
```

```text expected
id
```

```bash
sqlite3 /home/user/app/app.db "select id, name from users where active = 1 limit 5"
```

## Write a file only when it did not change meanwhile

Replace a text only when the file still has the content that was read earlier.

```nurt example
replace "debug" "info" in: ["/home/user/app/app.conf"] if_match: "1a2b3c4d"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.conf","type":"file","content_utf8":"level = debug\n"}]}
```

```text expected
E_STALE
```

## Write a Windows registry value

Write a DWORD value to the registry of the current user.

```nurt example
write "registry:/HKCU/Software/Example/Mode" "3" value_type: dword mode: overwrite
```

```text expected
registry:/HKCU/Software/Example/Mode
```

```powershell
Set-ItemProperty HKCU:\Software\Example -Name Mode -Value 3 -Type DWord
```

## Append a line to a log file

Append a line to the end of a file and keep its content.

```nurt example
write "/home/user/app/notes.txt" "done\n" mode: append ensure_newline: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/notes.txt","type":"file","content_utf8":"start"}]}
```

```text expected
/home/user/app/notes.txt
```

```bash
printf '\ndone\n' >> /home/user/app/notes.txt
```

## Create a file with Windows line endings

Create a batch file with CRLF line endings.

```nurt example
write "/home/user/app/run.cmd" "@echo off\necho run\n" mode: create line_ending: crlf
```

```text expected
/home/user/app/run.cmd
```

## Create a file in a missing directory

Create a file together with the directories of its path.

```nurt example
write "/home/user/app/out/logs/today.txt" "start\n" mode: create parents: true
```

```text expected
/home/user/app/out/logs/today.txt
```

```bash
mkdir -p /home/user/app/out/logs && printf 'start\n' > /home/user/app/out/logs/today.txt
```

## Write a JSON file

Write a JSON file; a document that is not valid JSON is rejected.

```nurt example
write "/home/user/app/settings.json" r#"{"debug": false}"# mode: create format: json
```

```text expected
/home/user/app/settings.json
```

## Replace a text in one file

Replace one text in one file.

```nurt example
replace "localhost" "db.internal" in: ["/home/user/app/app.conf"]
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.conf","type":"file","content_utf8":"host = localhost\n"}]}
```

```text expected
/home/user/app/app.conf
```

```bash
sed -i 's/localhost/db.internal/g' /home/user/app/app.conf
```

## Insert a line after the last line

Insert a line at the end of a file.

```nurt example
insert "exit 0" in: "/home/user/app/run.sh" position: end
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/run.sh","type":"file","content_utf8":"echo run\n"}]}
```

```text expected
/home/user/app/run.sh
```

## Delete a range of lines

Delete lines 10 to 12 of a file.

```nurt example
edit "/home/user/app/main.go" changes: [{delete_lines: 10..12}]
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\n"}]}
```

```text expected
/home/user/app/main.go
```

```bash
sed -i '10,12d' /home/user/app/main.go
```

## Edit a file only when the content matches

Apply a change only when the file still has the content that was read earlier.

```nurt example
edit "/home/user/app/main.go" changes: [{replace: "oldName(", with: "newName("}] if_match: "1a2b3c4d"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"oldName(1)\n"}]}
```

```text expected
E_STALE
```

## Remove a key of a configuration file

Remove one key of a JSON file.

```nurt example
set "/home/user/app/package.json" key: "scripts.lint" value: null
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/package.json","type":"file","content_utf8":"{\"scripts\":{\"lint\":\"eslint .\",\"test\":\"jest\"}}"}]}
```

```text expected
/home/user/app/package.json
```

## Set a value in an INI file

Set a value in an INI file and keep the other lines.

```nurt example
set "/home/user/app/app.ini" key: "database.host" value: "db.internal" format: ini
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.ini","type":"file","content_utf8":"[database]\nhost = localhost\n"}]}
```

```text expected
/home/user/app/app.ini
```

## Set a variable of an environment file

Change a variable of a dotenv file.

```nurt example
set "/home/user/app/.env" key: "LOG_LEVEL" value: "debug" format: env
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/.env","type":"file","content_utf8":"LOG_LEVEL=info\n"}]}
```

```text expected
/home/user/app/.env
```

## Read a key of a YAML file

Read one nested value of a YAML file.

```nurt example
read "/home/user/app/config.yaml" key: "server.port"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/config.yaml","type":"file","content_utf8":"server:\n  port: 8080\n"}]}
```

```text expected
8080
```

## Read lines of a file

Read lines 5 to 15 of a file with their numbers.

```nurt example
read "/home/user/app/main.go" lines: 5..15 line_numbers: true
```

```text expected
main.go
```

```bash
sed -n '5,15p' /home/user/app/main.go | cat -n
```

## Read the last lines of a file

Read the last twenty lines of a log.

```nurt example
read "/home/user/app/app.log" last: 20
```

```text expected
app.log
```

```bash
tail -n 20 /home/user/app/app.log
```

## Write to a SQLite database

Insert a row into a SQLite database.

```nurt example
sql "/home/user/app/app.db" "insert into users (name, active) values (?, ?)" params: ["ann", 1]
```

```text expected
/home/user/app/app.db
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
