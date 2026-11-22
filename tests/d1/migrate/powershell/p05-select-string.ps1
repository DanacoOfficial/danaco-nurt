# P05: Select-String with its switches.
Select-String -Pattern "error" -Path C:\logs\app.log
sls -Pattern 'oldName\(' -Path C:\src\*.go -CaseSensitive
Select-String -Pattern "GET /" -Path C:\logs\access.log -SimpleMatch -Context 2,3
Select-String -Pattern "deprecated" -Path C:\src\*.cs -NotMatch -List
