# P32: pipelines bound to variables and used later.
$logs = Get-ChildItem C:\logs -Filter *.log
$big = $logs | Where-Object { $_.Length -gt 10MB }
$big | Remove-Item
