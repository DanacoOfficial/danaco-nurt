# P28: JSON and CSV conversions.
Get-Content C:\data.json | ConvertFrom-Json
Import-Csv C:\data.csv
Get-Process | Export-Csv C:\out\processes.csv
Get-Service | ConvertTo-Json
