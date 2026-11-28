---
name: powershell-migration
description: Nawyki z PowerShell i sposób, w jaki Nurt je poprawia, tłumaczy lub odrzuca.
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

# migracja z PowerShell {#powershell-migration}

## Co jest poprawiane {#what-is-corrected}

Wiersz, który zaczyna się od cmdletu w postaci `Verb-Noun` lub od jego aliasu i którego argumenty mają postać `-Parameter value`, jest tłumaczony na program Nurt. Każdy tłumaczony cmdlet ma tabelę parametrów; parametr bez odpowiednika jest błędem, który wymienia parametry polecenia Nurt. Wynik pokazuje `corrected` i `ran`.

## Pliki i tekst {#files-and-text}

`Get-ChildItem -Path D` staje się `list "D"`. Z `-Recurse -Filter "*.go"` staje się `find "D" glob: "**/*.go"`. `Get-Content F -Tail N` staje się `read "F" last: N`. `Set-Content`, `Add-Content` i `Out-File` stają się `write` z `mode: overwrite` lub `mode: append`. `Select-String` domyślnie ignoruje wielkość liter, więc tłumaczenie dodaje `case_sensitive: false`.

`Copy-Item`, `Move-Item`, `Remove-Item`, `Rename-Item` i `New-Item` stają się `copy`, `move`, `remove`, `rename` i `create`. `Test-Path P` staje się `info "P"`, którego rekord ma pole `exists`.

```nurt
find "/home/user/app" glob: "**/*.go" |> filter size > 1mb |> sort size order: desc |> first 5
```

## Filtrowanie obiektów {#filtering-objects}

`Where-Object { $_.Length -gt 1MB }` staje się `filter size > 1mb`. Tłumaczone są tylko porównania pól, `-and`, `-or` i `-not`, z polami `Length`, `LastWriteTime`, `Name`, `FullName` i `Extension` odwzorowanymi na `size`, `modified`, `name`, `path` i `extension`.

`ForEach-Object { … }` nie ma odpowiednika. Wyjątkiem są bloki odczytujące jedną właściwość, takie jak `{ $_.Name }`, które stają się `select name`. Pętlę po rekordach, które coś zmieniają, zapisuje się przez `each` (zob. niżej); wartość obliczaną zapisuje się jako `select total: size * 2`.

`Group-Object` i `Measure-Object` stają się `group by:` i `summarize`. `X | Get-Member` staje się `X |> describe`.

## Rejestr, usługi i procesy {#registry-services-and-processes}

Ścieżka rejestru, taka jak `HKLM:\Software\X`, staje się ścieżką `"registry:/HKLM/Software/X"` i działa w `list`, `read` i `write`. `Get-Service`, `Start-Service` i `Stop-Service` stają się `services name: "N"` z następującym po nim `|> start` lub `|> stop`. `Get-Process` i `Stop-Process` stają się `processes` i `|> stop`.

## Podwyższanie uprawnień {#elevation}

`Start-Process -Verb RunAs`, opakowania podobne do `sudo` oraz uruchamianie konsoli jako administrator nie mają tłumaczenia tekstowego. Należy podać parametr wywołania `privilege: elevated` (lub `as_user` w systemach Linux i macOS) na etapie, który wymaga uprawnień; w systemie Windows backend `self` działa w tym samym procesie, gdy konsola ma podwyższone uprawnienia, a backend usługi pojawia się w wersji 0.1.2.

## Parametry wspólne {#common-parameters}

`-WhatIf` staje się parametrem wywołania `preview: true`. `-Confirm` jest usuwany z adnotacją, ponieważ Nurt nie prosi o potwierdzenie. `-ErrorAction SilentlyContinue` staje się `on_missing: skip` tam, gdzie polecenie ma ten parametr.

## Warunki, pętle, funkcje i parametry {#conditions-loops-functions-and-parameters}

Poprawki tłumaczą pojedyncze cmdlety. Konstrukcje skryptu zapisuje się w Nurt ręcznie, ponieważ są częścią języka, a nie poprawki jednego wiersza. Każda konstrukcja jest jednym wpisem dziennika i działa w trybie wszystko albo nic, czego nie dają `try`/`catch` ani `$ErrorActionPreference = "Stop"`.

| PowerShell | Nurt |
|---|---|
| `if (…) { … } elseif (…) { … } else { … }` | `when … { … } otherwise when … { … } otherwise { … }` |
| `Test-Path P` w warunku | `exists "P"` |
| `foreach ($f in Get-ChildItem …) { … }`, `… \| ForEach-Object { … }` ze zmianą | `each (find …) as f { … }`; rekord to `f`, pole to `f.path` |
| `while (…) { … }` | `while … limit: 20 { … }` |
| `do { … } until (…)` | `repeat until: … every: 5s limit: 2m { … }` |
| `break`, `continue` | `break`, `continue` wewnątrz pętli |
| `A; B; C` ze zmianami, które muszą się powieść albo zawieść razem | `steps { A  B  C }` |

## Parametry skryptu {#script-parameters}

Skrypt deklaruje swoje parametry przez `params`; `-WhatIf` staje się `preview`, jak w tym przykładzie.

| PowerShell | Nurt |
|---|---|
| `function F($Dir, $Keep = 7) { … }` | `define F(dir: path, keep: int = 7) { … }`; wywołanie to `F("/x", keep: 14)` |
| `param([string]$Dir, [int]$Days = 30, [switch]$Dry)` | `params { dir: path  days: int = 30  dry: bool = false }`; przełącznik jest `bool` zapisanym bez wartości (`dry`) |
| `$LASTEXITCODE`, `$?` | `exit_code` i pola `let` powiązanego z `run` |

```nurt
params {
    dir: path
    days: int = 30
    dry: bool = false
}
find @dir glob: "**/*.log" |> filter modified < ago(@days * 1d) |> remove preview: @dry
```

`nurt run cleanup.nurt dir: "D:/logs" days: 7 dry` przekazuje wartości, tak jak robi to `.\cleanup.ps1 -Dir D:\logs -Days 7 -Dry`. `-WhatIf` skryptu jest parametrem `preview` każdego etapu.

## Administracja {#administration}

Natywne polecenia grupy administracyjnej zastępują moduły systemu. Pełna lista, systemy i operacja odwrotna każdego polecenia znajdują się w [administracji](administration.md).

| PowerShell | Nurt |
|---|---|
| `New-LocalUser`, `Set-LocalUser`, `Remove-LocalUser` | `accounts.create`, `accounts.modify`, `accounts.remove` |
| `New-LocalGroup`, `Add-LocalGroupMember`, `Remove-LocalGroupMember` | `groups.create`, `groups.add_member`, `groups.remove_member` |
| `New-NetFirewallRule`, `Remove-NetFirewallRule` | `firewall.allow` lub `firewall.deny`, `firewall.remove` |
| `Set-DnsClientServerAddress`, `New-NetRoute` | `network.dns_set`, `network.route_add` |
| `Set-TimeZone`, `Set-Date` | `clock.timezone_set`, `clock.set` |
| `Restart-Computer`, `Stop-Computer` | `power.restart`, `power.shutdown` |
| `New-Partition`, `Format-Volume` | `disks.partition`, `disks.format` |
| `Invoke-CimMethod` | `windows.cim` z `invoke: <method>` |

## Co jest odrzucane {#what-is-rejected}

`Invoke-Expression` jest odrzucane: Nurt nie ma `eval`. `Set-Location` i `Push-Location` są odrzucane z wyjaśnieniem parametru wywołania `base` i parametru `cwd` polecenia `run`. Cmdlety wyjścia, takie jak `Write-Output X`, stają się `evaluate "X"`, a `Get-Date` staje się `evaluate now()`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
