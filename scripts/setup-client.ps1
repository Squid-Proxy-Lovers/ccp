$ErrorActionPreference = "Stop"

$ServerUrl = "http://192.168.130.34:1338"
$ClientKey = "ccp-client-7b6c2f915e4a8d30"
$InstallDir = if ($env:CCP_INSTALL_DIR) { $env:CCP_INSTALL_DIR } else { "$env:USERPROFILE\.local\bin" }

$arch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
    "X64" { "x86_64" }
    default { throw "The hosted Windows client currently supports x86_64 only" }
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$ClientPath = Join-Path $InstallDir "ccp-client.exe"
Invoke-WebRequest -UseBasicParsing -Uri "$ServerUrl/downloads/ccp-client-windows-$arch.exe" -OutFile $ClientPath
$UpdatePath = Join-Path $InstallDir "ccp-update.ps1"
Invoke-WebRequest -UseBasicParsing -Uri "$ServerUrl/ccp-update.ps1" -OutFile $UpdatePath
& $ClientPath subscribe-all

$McpVenv = Join-Path $env:USERPROFILE ".ccp-client\mcp-venv"
py -m venv $McpVenv
$McpPython = Join-Path $McpVenv "Scripts\python.exe"
& $McpPython -m pip install --upgrade --force-reinstall --no-cache-dir "$ServerUrl/downloads/ccp-mcp.tar.gz"
& $McpPython -c "from ccp_mcp_server.server import master_instructions"
$McpCommand = Join-Path $McpVenv "Scripts\ccp-mcp-server.exe"

if (Get-Command codex -ErrorAction SilentlyContinue) {
    & codex mcp remove ccp 2>$null
    & codex mcp add ccp --env "CCP_SERVER_URL=$ServerUrl" --env "CCP_CLIENT_KEY=$ClientKey" -- $McpCommand
    Write-Host "Configured Codex MCP."
}

if (Get-Command claude -ErrorAction SilentlyContinue) {
    & claude mcp remove ccp --scope user 2>$null
    & claude mcp add ccp --scope user --env "CCP_SERVER_URL=$ServerUrl" --env "CCP_CLIENT_KEY=$ClientKey" -- $McpCommand
    Write-Host "Configured Claude Code MCP."
}

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (($userPath -split ';') -notcontains $InstallDir) {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$InstallDir", "User")
}

Write-Host "Installed $ClientPath"
Write-Host "All open topics are connected automatically."
Write-Host "Update anytime:  powershell -File $UpdatePath"
Write-Host "Restart Codex or Claude Code after updating so it reloads the MCP tool list."
Write-Host "Discover topics: ccp-client remote-sessions"
