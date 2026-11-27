---
name: powershell-migration
description: Habits from PowerShell and how Nurt corrects, translates or rejects them.
commands: [list, find, read, write, search, copy, move, remove, create, rename, info, filter, select, sort, first, group, summarize, run, services, evaluate, describe, windows.cim]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/powershell-migration.md -->
<!-- Responsibility:   Documents powershell-migration. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

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

`ForEach-Object { … }` has no counterpart. The exceptions are blocks that read one property, such as `{ $_.Name }`, which become `select name`. A loop over records that change something is written with `each` (see below); a computed value is written as `select total: size * 2`.

`Group-Object` and `Measure-Object` become `group by:` and `summarize`. `X | Get-Member` becomes `X |> describe`.

## Registry, services and processes

A registry path such as `HKLM:\Software\X` becomes the path `"registry:/HKLM/Software/X"` and works in `list`, `read` and `write`. `Get-Service`, `Start-Service` and `Stop-Service` become `services name: "N"` followed by `|> start` or `|> stop`. `Get-Process` and `Stop-Process` become `processes` and `|> stop`.

## Elevation

`Start-Process -Verb RunAs`, `sudo`-like wrappers and running the console as the administrator have no text translation. Give the call parameter `privilege: elevated` (or `as_user` on Linux and macOS) on the stage that needs the rights; on Windows the backend `self` runs in the same process when the console is elevated, and the service backend arrives in 0.1.2.

## Common parameters

`-WhatIf` becomes the call parameter `preview: true`. `-Confirm` is removed with a note, because Nurt does not ask for confirmation. `-ErrorAction SilentlyContinue` becomes `on_missing: skip` where the command has the parameter.

## Conditions, loops, functions and parameters

The corrections translate single cmdlets. The constructs of a script are written in Nurt by hand, because they are part of the language and not of the correction of one line. Each construct is one journal entry and all or nothing, which `try`/`catch` and `$ErrorActionPreference = "Stop"` do not give.

| PowerShell | Nurt |
|---|---|
| `if (…) { … } elseif (…) { … } else { … }` | `when … { … } otherwise when … { … } otherwise { … }` |
| `Test-Path P` in a condition | `exists "P"` |
| `foreach ($f in Get-ChildItem …) { … }`, `… \| ForEach-Object { … }` with a change | `each (find …) as f { … }`; the record is `f`, a field is `f.path` |
| `while (…) { … }` | `while … limit: 20 { … }` |
| `do { … } until (…)` | `repeat until: … every: 5s limit: 2m { … }` |
| `break`, `continue` | `break`, `continue` inside a loop |
| `A; B; C` with changes that must stand or fall together | `steps { A  B  C }` |

## Script parameters

A script declares its parameters with `params`; `-WhatIf` becomes `preview`, as in this example.

| PowerShell | Nurt |
|---|---|
| `function F($Dir, $Keep = 7) { … }` | `define F(dir: path, keep: int = 7) { … }`; a call is `F("/x", keep: 14)` |
| `param([string]$Dir, [int]$Days = 30, [switch]$Dry)` | `params { dir: path  days: int = 30  dry: bool = false }`; a switch is a `bool` written without a value (`dry`) |
| `$LASTEXITCODE`, `$?` | `exit_code` and the fields of a `let` bound to `run` |

```nurt
params {
    dir: path
    days: int = 30
    dry: bool = false
}
find @dir glob: "**/*.log" |> filter modified < ago(@days * 1d) |> remove preview: @dry
```

`nurt run cleanup.nurt dir: "D:/logs" days: 7 dry` passes the values, as `.\cleanup.ps1 -Dir D:\logs -Days 7 -Dry` does. `-WhatIf` of a script is the parameter `preview` of every stage.

## Administration

The native commands of the administration group replace the modules of the system. The full list, the systems and the inverse operation of each command are in [administration](administration.md).

| PowerShell | Nurt |
|---|---|
| `New-LocalUser`, `Set-LocalUser`, `Remove-LocalUser` | `accounts.create`, `accounts.modify`, `accounts.remove` |
| `New-LocalGroup`, `Add-LocalGroupMember`, `Remove-LocalGroupMember` | `groups.create`, `groups.add_member`, `groups.remove_member` |
| `New-NetFirewallRule`, `Remove-NetFirewallRule` | `firewall.allow` or `firewall.deny`, `firewall.remove` |
| `Set-DnsClientServerAddress`, `New-NetRoute` | `network.dns_set`, `network.route_add` |
| `Set-TimeZone`, `Set-Date` | `clock.timezone_set`, `clock.set` |
| `Restart-Computer`, `Stop-Computer` | `power.restart`, `power.shutdown` |
| `New-Partition`, `Format-Volume` | `disks.partition`, `disks.format` |
| `Invoke-CimMethod` | `windows.cim` with `invoke: <method>` |

## What is rejected

`Invoke-Expression` is rejected: Nurt has no `eval`. `Set-Location` and `Push-Location` are rejected with an explanation of the call parameter `base` and the parameter `cwd` of `run`. Output cmdlets such as `Write-Output X` become `evaluate "X"` and `Get-Date` becomes `evaluate now()`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
