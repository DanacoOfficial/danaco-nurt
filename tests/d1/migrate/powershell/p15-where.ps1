# P15: Where-Object with field comparisons.
Get-ChildItem C:\logs | Where-Object { $_.Length -gt 1MB }
Get-ChildItem C:\logs | where { $_.LastWriteTime -lt (Get-Date).AddDays(-7) }
Get-ChildItem C:\cache | ? { $_.Extension -eq '.tmp' -and $_.Name -like 'cache*' }
