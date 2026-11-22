# Mixed: variables, loops, functions, evaluation and location changes.
$old = Get-ChildItem C:\logs -Recurse -Filter *.log | Where-Object { $_.LastWriteTime -lt (Get-Date).AddDays(-30) }
foreach ($file in $old) { Compress-Archive -Path $file.FullName -DestinationPath "$($file.FullName).zip" }
$old | Remove-Item -WhatIf
function Write-Log { param($Message) Add-Content C:\logs\cleanup.log $Message }
Set-Location C:\logs
Remove-Item .\tmp -Recurse; Write-Host "cleaned"
Invoke-Expression $extraCommand
if (Test-Path C:\logs\lock) { exit 1 }
