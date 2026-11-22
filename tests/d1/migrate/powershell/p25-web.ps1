# P25: Invoke-WebRequest and Invoke-RestMethod.
Invoke-WebRequest -Uri https://example.com/app.zip -OutFile C:\tmp\app.zip
iwr https://example.com -Method Head
Invoke-RestMethod -Uri https://api.example.com/items -Method Post -Body '{"a":1}'
irm https://api.example.com/items
