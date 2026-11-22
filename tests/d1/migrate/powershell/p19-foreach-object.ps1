# P19: ForEach-Object with a property and with a script block.
Get-ChildItem C:\logs | ForEach-Object { $_.Name }
Get-ChildItem C:\logs | % { Remove-Item $_.FullName }
