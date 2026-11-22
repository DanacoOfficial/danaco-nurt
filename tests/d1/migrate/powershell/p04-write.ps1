# P04: Set-Content, Add-Content and Out-File.
Set-Content C:\out\status.txt -Value "ready"
Add-Content -Path C:\out\log.txt -Value "line"
"done" | Out-File C:\out\done.txt
Get-Date | Out-File C:\out\log.txt -Append
