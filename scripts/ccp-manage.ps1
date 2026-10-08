$ErrorActionPreference = "Stop"
$ServerUrl = if ($env:CCP_SERVER_URL) { $env:CCP_SERVER_URL.TrimEnd('/') } else { "http://127.0.0.1:1338" }


if ($args.Count -eq 0) {
    Start-Process "$ServerUrl/admin"
    exit 0
}
if ($args.Count -ne 2 -or $args[0] -notin @("add", "delete", "stats")) {
    throw "Usage: ccp-manage.ps1 add|delete|stats SESSION"
}

if (-not $env:CCP_ADMIN_KEY) { throw "Set CCP_ADMIN_KEY to the server admin key" }
$AdminKey = $env:CCP_ADMIN_KEY
$commandName = $args[0]
$session = $args[1]
$headers = @{ "X-CCP-Admin-Key" = $AdminKey }

switch ($commandName) {
    "add" {
        Invoke-RestMethod -Method Post -Uri "$ServerUrl/v1/admin/sessions" `
            -Headers $headers -ContentType "application/json" `
            -Body (@{ session_name = $session } | ConvertTo-Json -Compress)
    }
    "delete" {
        $encoded = [Uri]::EscapeDataString($session)
        Invoke-RestMethod -Method Delete -Uri "$ServerUrl/v1/admin/sessions/$encoded" -Headers $headers
    }
    "stats" {
        $encoded = [Uri]::EscapeDataString($session)
        Invoke-RestMethod -Method Get -Uri "$ServerUrl/v1/admin/sessions/$encoded/stats" -Headers $headers
    }
}
