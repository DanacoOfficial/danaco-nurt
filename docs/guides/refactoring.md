---
name: refactoring
description: Renaming and changing code by syntax and symbols instead of by text.
commands: [code.symbols, code.references, code.rename, code.diagnostics, code.outline, search, replace, verify, run, assert]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# refactoring

## Text, syntax and symbols

`search` and `replace` work on text. `search mode: syntax` matches syntax patterns, so a match inside a comment or a string is not confused with code, and `scope: code` limits a text search to code. The `code.*` commands work on symbols: a symbol has a definition and references that a text search cannot tell apart from other uses of the same word.

## Finding symbols

`code.symbols` finds definitions by name across a repository. `kind` narrows the result to functions, methods, classes and other symbol kinds, and `match` selects exact, prefix, substring, fuzzy or regular expression comparison. `code.outline` lists the symbols of one file, and `code.references` lists the places that use a symbol.

```nurt
code.symbols "oldName" path: ["/srv/app/src"] kind: function
code.references "/srv/app/src/a.go" line: 12 column: 6
```

## Renaming

`code.rename` renames a symbol and its references in all files as one operation, so a single `undo` reverts it. The symbol comes from the input or from the arguments `path`, `line` and `column`. The parameter `source` selects the analysis: `lsp` uses an installed language server, `syntax` uses the local syntax analysis, and `auto` chooses the first that is available.

```nurt
code.symbols "oldName" path: ["/srv/app/src"] kind: function |> code.rename "newName"
```

## Checking after a change

A rename that compiles is not proof that the behavior is unchanged. `verify` runs a check after the change and rolls the change back when the check fails. `code.diagnostics` reports errors and warnings in changed files.

```nurt
code.rename "newName" path: "/srv/app/src/a.go" line: 12 column: 6 |> verify (run ["go", "build", "./..."] cwd: "/srv/app" |> assert exit_code: 0)
```

## Limits

A text replacement of a name changes every occurrence, including unrelated ones. When a text replacement is the only option, `check_references` reports references that would remain unchanged, and `assert paths: N` stops the program when the number of affected files differs from the expected one. Dynamic uses of a name, such as names built from strings, are not references and are not renamed.
