# P20: Compare-Object of two files.
Compare-Object (Get-Content C:\a.txt) (Get-Content C:\b.txt)
diff (gc C:\a.conf) (gc C:\b.conf)
