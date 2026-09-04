$ErrorActionPreference = "Stop"
$ServerUrl = "http://192.168.130.34:1338"
$AdminKey = "ccp-admin-f1a847d36c509e2b"

if ($args.Count -eq 0) {
    Start-Process "$ServerUrl/admin"
    exit 0
}
if ($args.Count -ne 2 -or $args[0] -notin @("add", "delete", "stats")) {
    throw "Usage: ccp-manage.ps1 add|delete|stats SESSION"
}

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
