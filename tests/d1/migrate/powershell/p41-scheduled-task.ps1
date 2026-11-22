# P41: scheduled tasks.
Register-ScheduledTask -TaskName Backup -Action $action -Trigger $trigger
schtasks /create /tn Backup /tr C:\bin\backup.cmd /sc daily
