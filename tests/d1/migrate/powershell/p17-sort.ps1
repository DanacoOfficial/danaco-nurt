# P17: Sort-Object ascending and descending.
Get-ChildItem C:\logs | Sort-Object -Property Length -Descending
Get-Process | Sort-Object CPU
