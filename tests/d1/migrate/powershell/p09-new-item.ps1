# P09: New-Item for directories, files and links.
New-Item -ItemType Directory -Path C:\out\reports
New-Item -ItemType File -Path C:\out\a.txt
New-Item -ItemType File -Path C:\out\b.txt -Value "x"
New-Item -ItemType SymbolicLink -Path C:\current -Target C:\releases\1.4
