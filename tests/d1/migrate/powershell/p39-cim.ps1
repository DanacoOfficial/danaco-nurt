# P39: Get-CimInstance.
Get-CimInstance -ClassName Win32_OperatingSystem
Get-CimInstance -ClassName Win32_LogicalDisk -Filter "DriveType=3"
