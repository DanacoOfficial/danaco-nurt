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
