# P16: Select-Object first, last, skip, properties and unique.
Get-Process | Select-Object -First 5
Get-ChildItem C:\logs | Select-Object -Property Name,Length
Get-Content C:\a.txt | Select-Object -Unique
Get-Content C:\a.txt | Select-Object -Last 3
Get-Content C:\a.txt | Select-Object -Skip 2
