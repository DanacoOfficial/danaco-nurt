# P18: Group-Object and Measure-Object.
Get-ChildItem C:\logs | Group-Object -Property Extension
Get-ChildItem C:\logs | Measure-Object -Property Length -Sum -Average -Maximum
Get-Content C:\a.txt | Measure-Object -Line -Word -Character
