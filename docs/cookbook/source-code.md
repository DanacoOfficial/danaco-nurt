---
category: source-code
title: Source code
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Source code

## Find the definition of a function

Find the symbols named `parseConfig` that are functions.

```nurt example
code.symbols "parseConfig" path: ["/home/user/app/src"] kind: function
```

```text expected
parseConfig
```

## Rename a function by syntax

Rename a function and its references; strings and comments are not changed.

```nurt example
code.symbols "oldName" path: ["/home/user/app/src"] kind: function |> code.rename "newName"
```

```text expected
newName
```

```bash
gopls rename -w /home/user/app/src/a.go:12:6 newName
```

## List the references of a symbol

List the references of the symbol at a position of a file.

```nurt example
code.references "/home/user/app/src/a.go" line: 12 column: 6
```

```text expected
/home/user/app/src/a.go
```

## Show the outline of a file

Show the symbols of a source file as an outline.

```nurt example
code.outline "/home/user/app/src/a.go"
```

```text expected
a.go
```

## Go to the definition at a position

Find the definition of the symbol at a position.

```nurt example
code.definition "/home/user/app/src/a.go" line: 40 column: 9
```

```text expected
/home/user/app/src
```

## Show the diagnostics of a project

Show the errors and warnings of the source files of a directory.

```nurt example
code.diagnostics ["/home/user/app/src"] severity: warning
```

```text expected
severity
```

## Find the implementations of an interface

Find the types that implement an interface method.

```nurt example
code.implementations "/home/user/app/src/shape.go" line: 5 column: 6
```

```text expected
/home/user/app/src
```

## Show the call hierarchy of a function

Show the functions that call a function.

```nurt example
code.call_hierarchy "/home/user/app/src/a.go" line: 12 column: 6 direction: incoming
```

```text expected
/home/user/app/src
```

## List the languages with a language server

List the languages for which code commands have an analyzer.

```nurt example
code.languages
```

```text expected
go
```

## Summarize a repository

Show the manifests, entry points and languages of a repository.

```nurt example
code.profile "/home/user/app" sections: ["manifests", "languages"]
```

```text expected
manifests
```

## Search code inside comments only

Search a pattern only in comments of source files.

```nurt example
search /deprecated/ "/home/user/app/src" scope: comments
```

```text expected
/home/user/app/src
```

## Replace only in code

Replace an identifier in code and leave strings and comments unchanged.

```nurt example
replace "oldName" "newName" in: ["/home/user/app/src"] glob: "**/*.go" scope: code whole_word: true
```

```text expected
/home/user/app/src
```

## Run the tests of a Go project

Run the tests of a project and require a zero exit code.

```nurt example
run ["go", "test", "./..."] cwd: "/home/user/app" |> assert exit_code: 0
```

```text expected
exit_code
```

## Check types and style of a project

Run the checks of the project analyzers on the changed files only.

```nurt example
analyze.check changed_only: true
```

```text expected
finding
```

## List the exported functions with their signatures

List the exported functions whose name starts with `New`.

```nurt example
code.symbols "New" match: prefix kind: function exported_only: true with_signature: true path: ["/home/user/app/src"]
```

```text expected
signature
```

## List the structures outside the tests

List the structures of a project and skip the test files.

```nurt example
code.symbols kind: struct path: ["/home/user/app/src"] exclude_preset: [tests]
```

```text expected
struct
```

## List the references within one file

List the references of a symbol in the file of its definition only.

```nurt example
code.references "/home/user/app/src/a.go" line: 12 column: 6 scope: file include_definition: true
```

```text expected
/home/user/app/src/a.go
```

## List the actions available for a range of lines

List the fixes and refactorings that the analyzer offers for lines 20 to 25.

```nurt example
code.actions "/home/user/app/src/a.go" line: 20 end_line: 25
```

```text expected
code_action
```

## Show the errors of the changed files

Show only the errors of the files that changed since the last commit.

```nurt example
code.diagnostics changed: true severity: error
```

```text expected
severity
```

## Show the entry points and the commands of a project

Show how a project is built, tested and started.

```nurt example
code.profile "/home/user/app" sections: ["commands", "entry_points"]
```

```text expected
commands
```

## Rename a symbol and its mentions in comments

Rename a function and change its name in comments of the code.

```nurt example
code.symbols "oldName" path: ["/home/user/app/src"] kind: function |> code.rename "newName" include_comments: true
```

```text expected
newName
```

## Search the strings of the source files

Find the web addresses written in the string literals of the code.

```nurt example
search /https?:\/\// "/home/user/app/src" scope: strings
```

```text expected
/home/user/app/src
```

## Check types and lint for the whole project

Run the type check and the linter and fail on errors only.

```nurt example
analyze.check ["/home/user/app/src"] checks: [types, lint] fail_on: error
```

```text expected
analysis
```

## Read the findings of an analysis

Read at most 20 errors of an earlier analysis.

```nurt example
analyze.show "a1" min_severity: error max_findings: 20
```

```text expected
finding
```

## Repeat the failed tests

Run again only the tests that failed in an earlier run.

```nurt example
test.failed "r1" failures: 10 message_lines: 5
```

```text expected
test_run
```
