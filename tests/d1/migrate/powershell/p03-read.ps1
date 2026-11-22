# P03: Get-Content with tail, count, raw and encoding.
Get-Content C:\logs\app.log -Tail 50
gc C:\notes.txt -TotalCount 10
Get-Content C:\data.json -Raw -Encoding UTF8
type C:\notes.txt
