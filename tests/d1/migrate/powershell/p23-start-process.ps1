# P23: Start-Process with a list and with a text of arguments.
Start-Process -FilePath git -ArgumentList "status","--short" -WorkingDirectory C:\proj -Wait -NoNewWindow
Start-Process -FilePath git -ArgumentList "status --short" -WorkingDirectory C:\proj -Wait
