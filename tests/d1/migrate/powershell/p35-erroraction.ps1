# P35: -ErrorAction.
Remove-Item C:\tmp\missing.txt -ErrorAction SilentlyContinue
Get-Item C:\a.txt -ErrorAction Stop
