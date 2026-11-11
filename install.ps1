#requires -Version 5.1
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later
# irm https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main/install.ps1 | iex
[CmdletBinding()]
param(
    [ValidateSet('stable', 'nightly')][string]$Channel = 'stable',
    [string]$Version = '',
    [string]$InstallDir = "$env:LOCALAPPDATA\ccp\bin"
)

$ErrorActionPreference = 'Stop'
$CcpRepository = 'Squid-Proxy-Lovers/ccp'
$CcpClientAsset = 'ccp-client-windows-x86_64.exe'

function Invoke-CcpNative {
    param([string]$Executable, [string[]]$ArgumentList)
    # Resolve first so a missing command cannot reuse a previous exit code.
    $tool = Get-Command $Executable -CommandType Application -ErrorAction Stop
    # Windows PowerShell can turn native stderr into terminating errors even
    # on success. Suppress tool output (which may contain private details) and
    # use the exit code for success, with a function-local error preference.
    $ErrorActionPreference = 'Continue'
    & $tool.Source @ArgumentList *> $null
    if ($LASTEXITCODE -ne 0) { throw 'A required external command failed.' }
}

function Get-CcpReleaseResponse {
    param([string]$Uri)
    Invoke-RestMethod -Uri $Uri
}

function Save-CcpReleaseFile {
    param([string]$Uri, [string]$OutFile)
    Invoke-WebRequest -UseBasicParsing -Uri $Uri -OutFile $OutFile
}

function Get-CcpReleaseTag {
    param([string]$ReleaseChannel, [string]$RequestedVersion)
    if ($RequestedVersion) {
        if ($ReleaseChannel -ne 'nightly' -or
            $RequestedVersion -cnotmatch '^nightly-\d{4}-\d{2}-\d{2}-[0-9a-f]{12}$') {
            throw 'Version requires Channel nightly and an exact dated nightly tag.'
        }
        return $RequestedVersion
    }
    $api = "https://api.github.com/repos/$CcpRepository/releases"
    if ($ReleaseChannel -eq 'stable') {
        $release = Get-CcpReleaseResponse -Uri "$api/latest"
        if ($release.draft -or $release.prerelease -or
            $release.tag_name -cnotmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$') {
            throw 'No valid stable release is available.'
        }
        return $release.tag_name
    }
    for ($page = 1; ; $page++) {
        $releases = @(Get-CcpReleaseResponse -Uri "${api}?per_page=100&page=$page")
        if ($releases.Count -eq 0) { throw 'No published nightly is available.' }
        $candidate = $releases | Where-Object {
            $_.prerelease -and -not $_.draft -and
            $_.tag_name -cmatch '^nightly-\d{4}-\d{2}-\d{2}-[0-9a-f]{12}$'
        } | Sort-Object published_at -Descending | Select-Object -First 1
        if ($candidate) { return $candidate.tag_name }
    }
}

function Get-CcpReleaseFile {
    param([string]$Tag, [string]$Stage)
    $base = "https://github.com/$CcpRepository/releases/download/$Tag"
    foreach ($name in @('ccp-release.json', 'ccp-provenance.json', $CcpClientAsset,
            'ccp-mcp.tar.gz', 'build-info.json')) {
        Save-CcpReleaseFile -Uri "$base/$name" -OutFile (Join-Path $Stage $name)
    }
}

function Confirm-CcpRelease {
    param([string]$Stage, [string]$ReleaseChannel, [string]$Tag)
    $manifestPath = Join-Path $Stage 'ccp-release.json'
    if ((Get-Item -LiteralPath $manifestPath).Length -gt 65536 -or
        (Get-Item -LiteralPath (Join-Path $Stage 'ccp-provenance.json')).Length -gt 4194304) {
        throw 'Release metadata exceeds the supported size.'
    }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($manifest.format -ne 1 -or $manifest.repository -cne $CcpRepository -or
        $manifest.source_sha -cnotmatch '^[0-9a-f]{40}$' -or
        $manifest.package_version -cnotmatch '^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$' -or
        $manifest.platforms -cnotcontains 'windows-x86_64') {
        throw 'Unsupported Windows release manifest.'
    }
    $ref = 'refs/heads/main'
    if ($ReleaseChannel -eq 'stable') {
        if ($Tag -cne "v$($manifest.package_version)") { throw 'Stable version mismatch.' }
        $ref = "refs/tags/$Tag"
    } elseif ($Tag -cnotmatch '^nightly-\d{4}-\d{2}-\d{2}-[0-9a-f]{12}$' -or
        -not $Tag.EndsWith('-' + $manifest.source_sha.Substring(0, 12))) {
        throw 'Nightly source mismatch.'
    }
    # Authenticate this exact manifest before trusting its artifact hashes. Match
    # the publisher policy in release_contract.py; there is no checksum fallback.
    Invoke-CcpNative 'gh' @('attestation', 'verify', $manifestPath,
        '--bundle', (Join-Path $Stage 'ccp-provenance.json'), '--hostname', 'github.com',
        '--repo', $CcpRepository, '--cert-identity',
        "https://github.com/$CcpRepository/.github/workflows/ci.yml@$ref",
        '--cert-oidc-issuer', 'https://token.actions.githubusercontent.com',
        '--source-ref', $ref, '--source-digest', $manifest.source_sha,
        '--signer-digest', $manifest.source_sha, '--deny-self-hosted-runners')
    foreach ($name in @($CcpClientAsset, 'ccp-mcp.tar.gz', 'build-info.json')) {
        $record = $manifest.artifacts.PSObject.Properties[$name]
        if (-not $record) { throw 'Required artifact is missing from the release manifest.' }
        $file = Get-Item -LiteralPath (Join-Path $Stage $name)
        $artifact = $record.Value
        if ($artifact.sha256 -cnotmatch '^[0-9a-f]{64}$' -or
            $file.Length -le 0 -or $file.Length -gt 268435456 -or
            $file.Length -ne $artifact.size -or
            (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() -cne $artifact.sha256) {
            throw 'Release artifact size or SHA-256 mismatch.'
        }
    }
}

function Publish-CcpFile {
    param([string]$Source, [string]$Destination)
    if (Test-Path -LiteralPath $Destination) {
        $item = Get-Item -LiteralPath $Destination -Force
        if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'Refusing to replace a directory or reparse point.'
        }
    }
    $stage = "$Destination.stage.$([Guid]::NewGuid().ToString('N'))"
    try {
        [IO.File]::Copy($Source, $stage, $false)
        if ([IO.File]::Exists($Destination)) {
            [IO.File]::Replace($stage, $Destination, [NullString]::Value)
        } else {
            [IO.File]::Move($stage, $Destination)
        }
    } finally {
        if ([IO.File]::Exists($stage)) { [IO.File]::Delete($stage) }
    }
}

function Install-CcpCommand {
    param([string]$Source, [string]$BinDirectory)
    $client = Join-Path $BinDirectory 'ccp-client.exe'
    $alias = Join-Path $BinDirectory 'ccp.exe'
    # Refresh only our own alias: its bytes must match the previous client.
    $owned = (Test-Path -LiteralPath $alias -PathType Leaf) -and
        (Test-Path -LiteralPath $client -PathType Leaf) -and
        ((Get-FileHash -LiteralPath $alias).Hash -eq (Get-FileHash -LiteralPath $client).Hash)
    $existing = Get-Command ccp -ErrorAction SilentlyContinue
    $available = -not (Test-Path -LiteralPath $alias) -and -not $existing
    Publish-CcpFile $Source $client
    if ($owned -or $available) {
        Publish-CcpFile $Source $alias
        Write-Information -InformationAction Continue 'Installed ccp and ccp-client.'
    } else {
        Write-Warning 'An existing ccp command was retained. Use ccp-client.'
    }
}

function Set-CcpUserPath {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$BinDirectory)
    if (-not $PSCmdlet.ShouldProcess($BinDirectory, 'Configure CCP')) { return }
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $BinDirectory) {
        $updated = (@($userPath -split ';' | Where-Object { $_ }) + $BinDirectory) -join ';'
        [Environment]::SetEnvironmentVariable('Path', $updated, 'User')
    }
    if (($env:Path -split ';') -notcontains $BinDirectory) {
        $env:Path = "$env:Path;$BinDirectory"
    }
}

function Install-CcpMcp {
    param([string]$Stage, [string]$ProfileDirectory)
    $venv = Join-Path $ProfileDirectory '.ccp-mcp\venv'
    $python = Join-Path $venv 'Scripts\python.exe'
    if (-not (Test-Path -LiteralPath $python)) {
        Invoke-CcpNative 'python' @('-m', 'venv', $venv)
    }
    Invoke-CcpNative $python @('-m', 'pip', 'install', '--quiet', '--upgrade',
        '--force-reinstall', (Join-Path $Stage 'ccp-mcp.tar.gz'))
    $command = Join-Path $venv 'Scripts\ccp-mcp-server.exe'
    if (-not (Test-Path -LiteralPath $command -PathType Leaf)) { throw 'MCP entry point is missing.' }
    return $command
}

function Set-CcpJsonHost {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$Path, [string]$McpCommand, [string]$Client)
    if (-not $PSCmdlet.ShouldProcess($Path, 'Configure CCP')) { return }
    $config = [PSCustomObject]@{}
    if (Test-Path -LiteralPath $Path) {
        $config = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    }
    if (-not $config.PSObject.Properties['mcpServers']) {
        $config | Add-Member -NotePropertyName mcpServers -NotePropertyValue ([PSCustomObject]@{})
    }
    if ($config.mcpServers.PSObject.Properties['ccp']) {
        Write-Warning 'Existing CCP MCP configuration retained; check its command and CCP_CLIENT_BIN.'
        return
    }
    $config.mcpServers | Add-Member -NotePropertyName ccp -NotePropertyValue @{
        command = $McpCommand; env = @{ CCP_CLIENT_BIN = $Client }
    }
    $directory = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $stage = Join-Path $directory "ccp-config-$([Guid]::NewGuid().ToString('N')).json"
    try {
        $json = $config | ConvertTo-Json -Depth 100 -WarningAction Stop
        [IO.File]::WriteAllText($stage, $json, [Text.UTF8Encoding]::new($false))
        Publish-CcpFile $stage $Path
    } finally {
        if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage }
    }
}

function Set-CcpMcpHost {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$ProfileDirectory, [string]$McpCommand, [string]$Client)
    if (-not $PSCmdlet.ShouldProcess($ProfileDirectory, 'Configure CCP')) { return }
    $hosts = @{
        Claude = (Join-Path $ProfileDirectory '.claude.json')
        Cursor = (Join-Path $ProfileDirectory '.cursor\mcp.json')
    }
    foreach ($name in @('Claude', 'Cursor')) {
        $path = $hosts[$name]
        $present = Test-Path -LiteralPath $path
        if ($name -eq 'Cursor') {
            $present = $present -or (Test-Path -LiteralPath (Join-Path $ProfileDirectory '.cursor')) -or
                (Test-Path -LiteralPath "$env:APPDATA\Cursor")
        }
        if (-not $present) { Write-Warning "$name MCP not configured: no host configuration found."; continue }
        try { Set-CcpJsonHost $path $McpCommand $Client; Write-Information -InformationAction Continue "$name MCP configuration checked." }
        catch { Write-Warning "$name MCP not configured: configuration could not be updated." }
    }
    $codexConfig = Join-Path $ProfileDirectory '.codex\config.toml'
    if (-not (Test-Path -LiteralPath $codexConfig)) {
        Write-Warning 'Codex MCP not configured: no host configuration found.'
        return
    }
    try {
        if ((Get-Content -LiteralPath $codexConfig -Raw) -match '(?m)^\s*\[mcp_servers\.ccp(?:\]|\.)') {
            Write-Warning 'Existing Codex CCP MCP configuration retained; check its paths.'
            return
        }
        # Let the host parse and write TOML; never interpolate Windows paths into it.
        Invoke-CcpNative 'codex' @('mcp', 'add', 'ccp', '--env', "CCP_CLIENT_BIN=$Client", '--', $McpCommand)
        Write-Information -InformationAction Continue 'Codex MCP configured.'
    } catch { Write-Warning 'Codex MCP not configured: codex mcp add failed or is unavailable.' }
}

function Install-CcpWindows {
    param([string]$ReleaseChannel, [string]$RequestedVersion, [string]$BinDirectory)
    if ($env:OS -ne 'Windows_NT' -or
        ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64' -and $env:PROCESSOR_ARCHITEW6432 -ne 'AMD64')) {
        throw 'This installer requires x86_64 Windows.'
    }
    if (-not $env:USERPROFILE -or -not $env:LOCALAPPDATA) { throw 'A Windows user profile is required.' }
    foreach ($tool in @('gh', 'python')) {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "Install $tool before running this installer." }
    }
    Invoke-CcpNative 'python' @('-c', 'import sys; sys.exit(sys.version_info < (3, 10))')
    $tag = Get-CcpReleaseTag $ReleaseChannel $RequestedVersion
    $stage = Join-Path $env:LOCALAPPDATA "ccp-download-$([Guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Path $stage | Out-Null
    try {
        Write-Information -InformationAction Continue 'Downloading and verifying the release; no software is installed until verification succeeds.'
        Get-CcpReleaseFile $tag $stage
        Confirm-CcpRelease $stage $ReleaseChannel $tag
        $bin = [IO.Path]::GetFullPath($BinDirectory)
        New-Item -ItemType Directory -Force -Path $bin | Out-Null
        Install-CcpCommand (Join-Path $stage $CcpClientAsset) $bin
        Set-CcpUserPath $bin
        Write-Information -InformationAction Continue "Installed verified Windows client from $tag."
        Write-Information -InformationAction Continue 'Installing the matching MCP bridge.'
        $mcp = Install-CcpMcp $stage $env:USERPROFILE
        Set-CcpMcpHost $env:USERPROFILE $mcp (Join-Path $bin 'ccp-client.exe')
        Write-Information -InformationAction Continue 'Installation complete. Open a new terminal and restart MCP hosts. Re-run the same command to update.'
    } finally {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
}

try { Install-CcpWindows $Channel $Version $InstallDir }
catch { throw 'CCP installation failed. Required tools, verified release assets, and writable user directories are needed. No unverified software was installed.' }
