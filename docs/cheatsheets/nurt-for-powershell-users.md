---
title: Nurt for PowerShell users
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
# Nurt for PowerShell users

Every row pairs a PowerShell command with its form in Nurt. The last column states whether Nurt corrects the habit automatically, rejects it with an error that carries the fix, or rejects it with an error that carries no program and asks for the program of the row.

The paths are those of a Windows host, and the call parameter `base` is `C:/Users/user`.

## Files and directories

| Rule | PowerShell | Nurt | Result |
|---|---|---|---|
| P01 | `Get-ChildItem -Path C:\Users\user\app` | `list "C:/Users/user/app" hidden: false` | corrected automatically |
| P02 | `Get-ChildItem C:\Users\user\app -Recurse -Filter *.go` | `find "C:/Users/user/app" glob: "**/*.go" type: any hidden: false` | corrected automatically |
| P06 | `Copy-Item C:\Users\user\app C:\Users\user\backup -Recurse` | `copy "C:/Users/user/app" to: "C:/Users/user/backup"` | corrected automatically |
| P07 | `Move-Item C:\Users\user\a.txt C:\Users\user\archive` | `move "C:/Users/user/a.txt" to: "C:/Users/user/archive" on_conflict: error` | corrected automatically |
| P08 | `Remove-Item C:\Users\user\app\build -Recurse -Force` | `remove "C:/Users/user/app/build"` | corrected automatically |
| P09 | `New-Item -ItemType Directory -Path C:\Users\user\app\out` | `create "C:/Users/user/app/out" type: directory` | corrected automatically |
| P10 | `Rename-Item C:\Users\user\a.txt -NewName b.txt` | `rename path: "C:/Users/user/a.txt" to: "b.txt"` | corrected automatically |
| P11 | `Test-Path C:\Users\user\a.txt` | `info "C:/Users/user/a.txt"` | corrected automatically |

## Reading and searching text

| Rule | PowerShell | Nurt | Result |
|---|---|---|---|
| P03 | `Get-Content C:\Users\user\a.log -Tail 20` | `read "C:/Users/user/a.log" last: 20` | corrected automatically |
| P05 | `Select-String -Pattern todo -Path C:\Users\user\app\a.go` | `search /todo/ "C:/Users/user/app/a.go" case_sensitive: false` | corrected automatically: Select-String ignores letter case by default, so the pattern is case-insensitive |
| P21 | `Get-FileHash C:\Users\user\a.zip -Algorithm SHA256` | `hash "C:/Users/user/a.zip" algorithm: sha256` | corrected automatically |
| P20 | `Compare-Object (Get-Content a.txt) (Get-Content b.txt)` | `diff "C:/Users/user/a.txt" "C:/Users/user/b.txt"` | corrected automatically: only files are compared; relative paths are resolved against the call parameter base |

## Writing text

| Rule | PowerShell | Nurt | Result |
|---|---|---|---|
| P04 | `Add-Content C:\Users\user\app\.gitignore -Value dist/` | `write "C:/Users/user/app/.gitignore" "dist/\n" mode: append` | corrected automatically |

## Registry

| Rule | PowerShell | Nurt | Result |
|---|---|---|---|
| P12 | `Get-ItemProperty -Path HKCU:\Software\Example -Name Mode` | `read "registry:/HKCU/Software/Example/Mode"` | corrected automatically |
| P13 | `Set-ItemProperty -Path HKCU:\Software\Example -Name Mode -Value 3 -Type DWord` | `write "registry:/HKCU/Software/Example/Mode" "3" value_type: dword mode: overwrite` | corrected automatically |

## Pipelines

| Rule | PowerShell | Nurt | Result |
|---|---|---|---|
| P15 | `Get-ChildItem C:\Users\user\app -Recurse \| Where-Object { $_.Length -gt 1MB }` | `find "C:/Users/user/app" type: any hidden: false \|> filter size > 1mib` | corrected automatically |
| P16 | `Get-Process \| Select-Object -First 5` | `processes \|> first 5` | corrected automatically |
| P17 | `Get-Process \| Sort-Object WorkingSet -Descending` | `processes \|> sort memory order: desc` | corrected automatically |
| P18 | `Get-Process \| Measure-Object` | `processes \|> count` | corrected automatically |
| P19 | `Get-Process \| ForEach-Object { Write-Host $_.Name }` | `processes \|> select name` | error without fix: loops are replaced by pipelines; write the pipeline that selects the field |

## Processes and services

| Rule | PowerShell | Nurt | Result |
|---|---|---|---|
| P22 | `Stop-Process -Id 8120 -Force` | `processes pid: 8120 \|> stop signal: kill` | corrected automatically |
| P23 | `Start-Process npm -ArgumentList 'test' -WorkingDirectory C:\Users\user\app -Wait` | `run ["npm", "test"] cwd: "C:/Users/user/app" wait: forever` | corrected automatically |
| P24 | `Restart-Service nginx` | `services name: "nginx" \|> restart` | corrected automatically |
| P31 | `$env:PATH` | `environment name: "PATH"` | corrected automatically |
| P40 | `Get-Volume` | `disks` | corrected automatically |
| P37 | `Get-WinEvent -LogName System -MaxEvents 50` | `logs source: "System" limit: 50` | corrected automatically |
| P39 | `Get-CimInstance -ClassName Win32_OperatingSystem` | `windows.cim "Win32_OperatingSystem"` | corrected automatically |

## Network

| Rule | PowerShell | Nurt | Result |
|---|---|---|---|
| P25 | `Invoke-WebRequest https://example.com/a.zip -OutFile C:\Users\user\a.zip` | `http.download "https://example.com/a.zip" "C:/Users/user/a.zip"` | corrected automatically |
| P38 | `Test-Connection example.com` | `network.ping "example.com"` | corrected automatically |
| P38b | `Resolve-DnsName example.com` | `network.resolve "example.com"` | corrected automatically |

## Archives

| Rule | PowerShell | Nurt | Result |
|---|---|---|---|
| P29 | `Expand-Archive -Path C:\Users\user\a.zip -DestinationPath C:\Users\user\data` | `archive.extract "C:/Users/user/a.zip" into: "C:/Users/user/data" on_conflict: error` | corrected automatically |

## Other

| Rule | PowerShell | Nurt | Result |
|---|---|---|---|
| P26 | `Start-Sleep -Seconds 5` | `sleep 5s` | corrected automatically |
| P30 | `Set-Location C:\Users\user\app` | `run ["make"] cwd: "C:/Users/user/app"` | error without fix: there is no current directory; pass the directory of the next program as cwd or base |
| P42 | `Invoke-Expression $command` | `run ["pwsh", "-Command", "Get-Date"] cwd: "C:/Users/user"` | error without fix: the language has no eval; run the program explicitly |

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
