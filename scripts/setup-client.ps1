$ErrorActionPreference = "Stop"

$ServerUrl = if ($env:CCP_SERVER_URL) { $env:CCP_SERVER_URL.TrimEnd('/') } else { "http://127.0.0.1:1338" }
if (-not $env:CCP_CLIENT_KEY) { throw "Set CCP_CLIENT_KEY to the server client key" }
$ClientKey = $env:CCP_CLIENT_KEY
$env:CCP_SERVER_URL = $ServerUrl
$InstallDir = if ($env:CCP_INSTALL_DIR) { $env:CCP_INSTALL_DIR } else { "$env:USERPROFILE\.local\bin" }

function Assert-NativeSuccess([string] $Operation) {
    if ($LASTEXITCODE -ne 0) { throw "$Operation failed (exit $LASTEXITCODE)" }
}

$arch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
    "X64" { "x86_64" }
    default { throw "The hosted Windows client currently supports x86_64 only" }
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$ClientPath = Join-Path $InstallDir "ccp-client.exe"
$UpdatePath = Join-Path $InstallDir "ccp-update.ps1"
$Staging = Join-Path $InstallDir ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $Staging | Out-Null
try {
    $StagedClient = Join-Path $Staging "ccp-client.exe"
    $StagedUpdate = Join-Path $Staging "ccp-update.ps1"
    Invoke-WebRequest -UseBasicParsing -Uri "$ServerUrl/downloads/ccp-client-windows-$arch.exe" -OutFile $StagedClient
    Invoke-WebRequest -UseBasicParsing -Uri "$ServerUrl/ccp-update.ps1" -OutFile $StagedUpdate
    Move-Item -Force -Path $StagedClient -Destination $ClientPath
    Move-Item -Force -Path $StagedUpdate -Destination $UpdatePath
} finally {
    Remove-Item -Recurse -Force -Path $Staging
}
& $ClientPath subscribe-all --server $ServerUrl
Assert-NativeSuccess "Subscribe"

$McpVenv = Join-Path $env:USERPROFILE ".ccp-client\mcp-venv"
py -m venv $McpVenv
Assert-NativeSuccess "Create MCP environment"
$McpPython = Join-Path $McpVenv "Scripts\python.exe"
& $McpPython -m pip install --upgrade --force-reinstall --no-cache-dir "$ServerUrl/downloads/ccp-mcp.tar.gz"
Assert-NativeSuccess "Install MCP package"
& $McpPython -c "from ccp_mcp_server.server import master_instructions"
Assert-NativeSuccess "Import MCP package"
$McpCommand = Join-Path $McpVenv "Scripts\ccp-mcp-server.exe"

if (Get-Command codex -ErrorAction SilentlyContinue) {
    & codex mcp remove ccp 2>$null
    & codex mcp add ccp --env "CCP_SERVER_URL=$ServerUrl" --env "CCP_CLIENT_KEY=$ClientKey" --env "CCP_CLIENT_BIN=$ClientPath" -- $McpCommand
    Assert-NativeSuccess "Configure MCP host"
    Write-Host "Configured Codex MCP."
}

if (Get-Command claude -ErrorAction SilentlyContinue) {
    & claude mcp remove ccp --scope user 2>$null
    & claude mcp add ccp --scope user --env "CCP_SERVER_URL=$ServerUrl" --env "CCP_CLIENT_KEY=$ClientKey" --env "CCP_CLIENT_BIN=$ClientPath" -- $McpCommand
    Assert-NativeSuccess "Configure MCP host"
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
