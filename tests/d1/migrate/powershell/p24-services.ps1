# P24: services.
Get-Service -Name Spooler
Start-Service -Name Spooler
Stop-Service Spooler
Restart-Service -Name W3SVC
Set-Service -Name W3SVC -StartupType Automatic
