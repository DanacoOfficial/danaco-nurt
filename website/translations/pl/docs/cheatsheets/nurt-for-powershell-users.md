---
title: Nurt dla użytkowników PowerShell
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cheatsheets/nurt-for-powershell-users.md -->
<!-- Responsibility:   Documents Nurt for PowerShell users. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Nurt dla użytkowników PowerShell {#nurt-for-powershell-users}

Każdy wiersz zestawia polecenie PowerShell z jego postacią w Nurt. Ostatnia kolumna podaje, czy Nurt poprawia nawyk automatycznie, odrzuca go błędem zawierającym poprawkę, czy odrzuca go błędem bez programu i prosi o program z danego wiersza.

Ścieżki pochodzą z hosta z systemem Windows, a parametr wywołania `base` ma wartość `C:/Users/user`.

## Pliki i katalogi {#files-and-directories}

| Reguła | PowerShell | Nurt | Wynik |
|---|---|---|---|
| P01 | `Get-ChildItem -Path C:\Users\user\app` | `list "C:/Users/user/app" hidden: false` | poprawiane automatycznie |
| P02 | `Get-ChildItem C:\Users\user\app -Recurse -Filter *.go` | `find "C:/Users/user/app" glob: "**/*.go" type: any hidden: false` | poprawiane automatycznie |
| P06 | `Copy-Item C:\Users\user\app C:\Users\user\backup -Recurse` | `copy "C:/Users/user/app" to: "C:/Users/user/backup"` | poprawiane automatycznie |
| P07 | `Move-Item C:\Users\user\a.txt C:\Users\user\archive` | `move "C:/Users/user/a.txt" to: "C:/Users/user/archive" on_conflict: error` | poprawiane automatycznie |
| P08 | `Remove-Item C:\Users\user\app\build -Recurse -Force` | `remove "C:/Users/user/app/build"` | poprawiane automatycznie |
| P09 | `New-Item -ItemType Directory -Path C:\Users\user\app\out` | `create "C:/Users/user/app/out" type: directory` | poprawiane automatycznie |
| P10 | `Rename-Item C:\Users\user\a.txt -NewName b.txt` | `rename path: "C:/Users/user/a.txt" to: "b.txt"` | poprawiane automatycznie |
| P11 | `Test-Path C:\Users\user\a.txt` | `info "C:/Users/user/a.txt"` | poprawiane automatycznie |

## Odczyt i wyszukiwanie tekstu {#reading-and-searching-text}

| Reguła | PowerShell | Nurt | Wynik |
|---|---|---|---|
| P03 | `Get-Content C:\Users\user\a.log -Tail 20` | `read "C:/Users/user/a.log" last: 20` | poprawiane automatycznie |
| P05 | `Select-String -Pattern todo -Path C:\Users\user\app\a.go` | `search /todo/ "C:/Users/user/app/a.go" case_sensitive: false` | poprawiane automatycznie: Select-String domyślnie ignoruje wielkość liter, więc wzorzec nie rozróżnia wielkości liter |
| P21 | `Get-FileHash C:\Users\user\a.zip -Algorithm SHA256` | `hash "C:/Users/user/a.zip" algorithm: sha256` | poprawiane automatycznie |
| P20 | `Compare-Object (Get-Content a.txt) (Get-Content b.txt)` | `diff "C:/Users/user/a.txt" "C:/Users/user/b.txt"` | poprawiane automatycznie: porównywane są tylko pliki; ścieżki względne są rozwiązywane względem parametru wywołania base |

## Zapis tekstu {#writing-text}

| Reguła | PowerShell | Nurt | Wynik |
|---|---|---|---|
| P04 | `Add-Content C:\Users\user\app\.gitignore -Value dist/` | `write "C:/Users/user/app/.gitignore" "dist/\n" mode: append` | poprawiane automatycznie |

## Rejestr {#registry}

| Reguła | PowerShell | Nurt | Wynik |
|---|---|---|---|
| P12 | `Get-ItemProperty -Path HKCU:\Software\Example -Name Mode` | `read "registry:/HKCU/Software/Example/Mode"` | poprawiane automatycznie |
| P13 | `Set-ItemProperty -Path HKCU:\Software\Example -Name Mode -Value 3 -Type DWord` | `write "registry:/HKCU/Software/Example/Mode" "3" value_type: dword mode: overwrite` | poprawiane automatycznie |

## Potoki {#pipelines}

| Reguła | PowerShell | Nurt | Wynik |
|---|---|---|---|
| P15 | `Get-ChildItem C:\Users\user\app -Recurse \| Where-Object { $_.Length -gt 1MB }` | `find "C:/Users/user/app" type: any hidden: false \|> filter size > 1mib` | poprawiane automatycznie |
| P16 | `Get-Process \| Select-Object -First 5` | `processes \|> first 5` | poprawiane automatycznie |
| P17 | `Get-Process \| Sort-Object WorkingSet -Descending` | `processes \|> sort memory order: desc` | poprawiane automatycznie |
| P18 | `Get-Process \| Measure-Object` | `processes \|> count` | poprawiane automatycznie |
| P19 | `Get-Process \| ForEach-Object { Write-Host $_.Name }` | `processes \|> select name` | błąd bez poprawki: pętle są zastępowane potokami; należy zapisać potok wybierający pole |

## Procesy i usługi {#processes-and-services}

| Reguła | PowerShell | Nurt | Wynik |
|---|---|---|---|
| P22 | `Stop-Process -Id 8120 -Force` | `processes pid: 8120 \|> stop signal: kill` | poprawiane automatycznie |
| P23 | `Start-Process npm -ArgumentList 'test' -WorkingDirectory C:\Users\user\app -Wait` | `run ["npm", "test"] cwd: "C:/Users/user/app" wait: forever` | poprawiane automatycznie |
| P24 | `Restart-Service nginx` | `services name: "nginx" \|> restart` | poprawiane automatycznie |
| P31 | `$env:PATH` | `environment name: "PATH"` | poprawiane automatycznie |
| P40 | `Get-Volume` | `disks` | poprawiane automatycznie |
| P37 | `Get-WinEvent -LogName System -MaxEvents 50` | `logs source: "System" limit: 50` | poprawiane automatycznie |
| P39 | `Get-CimInstance -ClassName Win32_OperatingSystem` | `windows.cim "Win32_OperatingSystem"` | poprawiane automatycznie |

## Sieć {#network}

| Reguła | PowerShell | Nurt | Wynik |
|---|---|---|---|
| P25 | `Invoke-WebRequest https://example.com/a.zip -OutFile C:\Users\user\a.zip` | `http.download "https://example.com/a.zip" "C:/Users/user/a.zip"` | poprawiane automatycznie |
| P38 | `Test-Connection example.com` | `network.ping "example.com"` | poprawiane automatycznie |
| P38b | `Resolve-DnsName example.com` | `network.resolve "example.com"` | poprawiane automatycznie |

## Archiwa {#archives}

| Reguła | PowerShell | Nurt | Wynik |
|---|---|---|---|
| P29 | `Expand-Archive -Path C:\Users\user\a.zip -DestinationPath C:\Users\user\data` | `archive.extract "C:/Users/user/a.zip" into: "C:/Users/user/data" on_conflict: error` | poprawiane automatycznie |

## Pozostałe {#other}

| Reguła | PowerShell | Nurt | Wynik |
|---|---|---|---|
| P26 | `Start-Sleep -Seconds 5` | `sleep 5s` | poprawiane automatycznie |
| P30 | `Set-Location C:\Users\user\app` | `run ["make"] cwd: "C:/Users/user/app"` | błąd bez poprawki: nie ma katalogu bieżącego; należy przekazać katalog następnego programu jako cwd lub base |
| P42 | `Invoke-Expression $command` | `run ["pwsh", "-Command", "Get-Date"] cwd: "C:/Users/user"` | błąd bez poprawki: język nie ma eval; należy uruchomić program jawnie |

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
