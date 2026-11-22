# P37: event logs.
Get-EventLog -LogName System -Newest 20
Get-WinEvent -LogName Application -MaxEvents 50
