---
category: edit-single-file
title: Editing a single file
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
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
