---
name: powershell-migration
description: Habits from PowerShell and how Nurt corrects, translates or rejects them.
commands: [list, find, read, write, search, copy, move, remove, create, rename, info, filter, select, sort, first, group, summarize, run, services, evaluate, describe]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# powershell-migration

## What is corrected

A line that starts with a cmdlet in the form `Verb-Noun`, or with an alias of one, and whose arguments have the form `-Parameter value` is translated to a Nurt program. Every translated cmdlet has a table of parameters; a parameter without a counterpart is an error that lists the parameters of the Nurt command. The result shows `corrected` and `ran`.

## Files and text

`Get-ChildItem -Path D` becomes `list "D"`. With `-Recurse -Filter "*.go"` it becomes `find "D" glob: "**/*.go"`. `Get-Content F -Tail N` becomes `read "F" last: N`. `Set-Content`, `Add-Content` and `Out-File` become `write` with `mode: overwrite` or `mode: append`. `Select-String` ignores case by default, so the translation adds `case_sensitive: false`.

`Copy-Item`, `Move-Item`, `Remove-Item`, `Rename-Item` and `New-Item` become `copy`, `move`, `remove`, `rename` and `create`. `Test-Path P` becomes `info "P"`, whose record has the field `exists`.

```nurt
find "/home/user/app" glob: "**/*.go" |> filter size > 1mb |> sort size order: desc |> first 5
```

## Filtering objects

`Where-Object { $_.Length -gt 1MB }` becomes `filter size > 1mb`. Only comparisons of fields, `-and`, `-or` and `-not` are translated, with the fields `Length`, `LastWriteTime`, `Name`, `FullName` and `Extension` mapped to `size`, `modified`, `name`, `path` and `extension`.

`ForEach-Object { … }` has no counterpart. The exceptions are blocks that read one property, such as `{ $_.Name }`, which become `select name`. Loops are replaced by patterns and pipelines; a computed value is written as `select total: size * 2`.

`Group-Object` and `Measure-Object` become `group by:` and `summarize`. `X | Get-Member` becomes `X |> describe`.

## Registry, services and processes

A registry path such as `HKLM:\Software\X` becomes the path `"registry:/HKLM/Software/X"` and works in `list`, `read` and `write`. `Get-Service`, `Start-Service` and `Stop-Service` become `services name: "N"` followed by `|> start` or `|> stop`. `Get-Process` and `Stop-Process` become `processes` and `|> stop`.

## Common parameters

`-WhatIf` becomes the call parameter `preview: true`. `-Confirm` is removed with a note, because Nurt does not ask for confirmation. `-ErrorAction SilentlyContinue` becomes `on_missing: skip` where the command has the parameter.

## What is rejected

`Invoke-Expression` is rejected: Nurt has no `eval`. `Set-Location` and `Push-Location` are rejected with an explanation of the call parameter `base` and the parameter `cwd` of `run`. Output cmdlets such as `Write-Output X` become `evaluate "X"` and `Get-Date` becomes `evaluate now()`.
