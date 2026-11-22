# P42: call operator, Invoke-Expression and Invoke-Command.
& git status
& "C:\Program Files\Git\bin\git.exe" log -5
Invoke-Expression "Get-Date"
Invoke-Command -ComputerName srv01 -ScriptBlock { Get-Process }
