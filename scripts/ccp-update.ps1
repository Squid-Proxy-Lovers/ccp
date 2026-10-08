$ErrorActionPreference = "Stop"
$ServerUrl = if ($env:CCP_SERVER_URL) { $env:CCP_SERVER_URL.TrimEnd('/') } else { "http://127.0.0.1:1338" }
$env:CCP_SERVER_URL = $ServerUrl
Invoke-Expression (Invoke-RestMethod -Uri "$ServerUrl/setup-client.ps1")
