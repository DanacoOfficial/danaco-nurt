---
title: First steps
order: 3
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/first-steps.md -->
<!-- Responsibility:   Documents First steps. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# First steps

## Running a program

`nurt` takes a program as its argument. A program is a command with arguments, optionally followed by more commands joined with `|>`.

```nurt example
find "/home/user/app" glob: "**/*.go" |> filter size > 1mb |> sort size order: desc |> first 5
```

The command `find` produces `entry` records, one for each file. `filter`, `sort` and `first` consume and produce records of the same kind. The text of the result lists the remaining records.

## Paths and arguments

A path is a quoted text: `"/home/user/app"` or `"C:/Users/user/app"`. A regular expression is written between slashes: `/oldName\(/`. A name followed by a colon introduces a named argument: `glob: "**/*.go"`. Positional arguments follow the order of the parameters on the page of the command.

## Checking before running

`nurt check` parses a program, applies the corrections of the language and binds it to the catalog. It prints the canonical form and the effect, and changes nothing.

```text
nurt check -e 'remove "/home/user/app/build"'
```

The call parameter `preview: true` runs a changing program without changing anything and reports the targets.

## Reading the report

A program that changes files returns an effect report: the status, the number of the operation in the journal, the targets, the changes and any unusual effects. [Effect report](effect-report.md) lists the fields.

```nurt example
replace /oldName\(/ "newName(" in: ["/home/user/app/src"] glob: "**/*.go"
```

## Undoing a change

Every operation that changes files is recorded in the journal. `history` lists the operations and `undo` reverses one of them.

```nurt example
undo 18
```

See [Journal and undo](journal.md) for conflicts, retention and the operations that cannot be reversed.

## Getting help

`help` without arguments lists the commands. `help <command>` shows the synopsis, the parameters, the record kinds, the effect and examples.

```nurt example
help "replace"
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
