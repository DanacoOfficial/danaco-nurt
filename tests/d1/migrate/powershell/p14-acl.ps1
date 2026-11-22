# P14: access lists and attributes.
Get-Acl C:\data
icacls C:\data
attrib +r C:\data\a.txt
Get-Acl C:\a | Set-Acl C:\b
