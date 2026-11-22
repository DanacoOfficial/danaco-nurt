# Removes large Go sources and copies the result.
$files = Get-ChildItem -Path C:\proj -Recurse -Filter *.go
$files | Where-Object { $_.Length -gt 1MB } | Remove-Item

Set-Location C:\proj
Remove-Item .\build -Recurse -Force; Write-Host "done"
Copy-Item -Path C:\proj\a.txt -Destination C:\backup && Write-Output "copied"

foreach ($f in $files) { Remove-Item $f.FullName }
function Get-Size { param($p) (Get-Item $p).Length }
Invoke-Expression "Get-Date"
. .\settings.ps1

<# Starts the version control status
   check in the project directory. #>
Start-Process -FilePath git -ArgumentList "status --short" -Wait
exit 1
