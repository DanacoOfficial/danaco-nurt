# P02: recursive Get-ChildItem with filters.
Get-ChildItem -Path C:\proj -Recurse -Filter *.go
Get-ChildItem C:\proj -Recurse -File -Depth 2
Get-ChildItem C:\logs -Recurse -Include *.log,*.txt -Force -Name
