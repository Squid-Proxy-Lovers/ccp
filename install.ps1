#requires -Version 5.1
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later
# Fetch this bootstrap from an immutable reviewed commit; see README.md.
[CmdletBinding()]
param(
    [ValidateSet('stable', 'nightly')][string]$Channel = 'stable',
    [string]$Version = '',
    [string]$InstallDir = "$env:LOCALAPPDATA\ccp\bin"
)

$ErrorActionPreference = 'Stop'
$CcpRepository = 'Squid-Proxy-Lovers/ccp'
$CcpClientAsset = 'ccp-client-windows-x86_64.exe'

function Test-CcpPythonAlias {
    param([string]$Path)
    if (-not $env:LOCALAPPDATA) { return $false }
    $aliases = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps'))
    $prefix = $aliases.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    return [IO.Path]::GetFullPath($Path).StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
}

function Resolve-CcpApplication {
    param([string]$Executable)
    $python = (Split-Path -Leaf $Executable) -imatch '^python(?:[0-9]+(?:\.[0-9]+)*)?(?:\.exe)?$'
    $tool = Get-Command $Executable -CommandType Application -ErrorAction SilentlyContinue |
        Where-Object { -not $python -or -not (Test-CcpPythonAlias $_.Source) } |
        Select-Object -First 1
    if (-not $tool) {
        $message = if ($python) {
            'Install Python 3.10 or newer and add its real interpreter to PATH; Microsoft Store app-execution aliases are not supported.'
        } else { "Required executable '$Executable' was not found on PATH." }
        $failure = [InvalidOperationException]::new($message)
        $failure.Data['CcpPrerequisite'] = $message
        throw $failure
    }
    return $tool.Source
}

function Resolve-CcpPython {
    $python = Resolve-CcpApplication 'python'
    try { Invoke-CcpNative $python @('-c', 'import sys; sys.exit(sys.version_info < (3, 10))') }
    catch {
        $message = 'Install Python 3.10 or newer with venv support and add its real interpreter to PATH.'
        $failure = [InvalidOperationException]::new($message)
        $failure.Data['CcpPrerequisite'] = $message
        throw $failure
    }
    return $python
}

function Invoke-CcpNative {
    param([string]$Executable, [string[]]$ArgumentList)
    # Resolve first so a missing command cannot reuse a previous exit code.
    $tool = Resolve-CcpApplication $Executable
    # Windows PowerShell can turn native stderr into terminating errors even
    # on success. Suppress tool output (which may contain private details) and
    # use the exit code for success, with a function-local error preference.
    $ErrorActionPreference = 'Continue'
    # Disable PowerShell 7's native-error conversion locally; the explicit
    # exit-code check below always throws on failure, whatever the caller chose.
    $PSNativeCommandUseErrorActionPreference = $false
    & $tool @ArgumentList *> $null
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) { throw 'A required external command failed.' }
}

function Confirm-CcpPath {
    param([string]$Path, [switch]$AllowMissing)
    $root = [IO.Path]::GetFullPath($env:USERPROFILE).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $full = [IO.Path]::GetFullPath($Path)
    $prefix = $root + [IO.Path]::DirectorySeparatorChar
    if ($full -ne $root -and -not $full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Installer paths must stay under the current user profile.'
    }
    $current = $root
    $components = @('') + @($full.Substring($root.Length).Split([char[]]@('/', '\'), [StringSplitOptions]::RemoveEmptyEntries))
    foreach ($component in $components) {
        if ($component) { $current = Join-Path $current $component }
        try { $item = Get-Item -LiteralPath $current -Force -ErrorAction Stop }
        catch [Management.Automation.ItemNotFoundException] {
            if ($AllowMissing) { return }
            throw
        }
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'Installer paths cannot contain reparse points.'
        }
    }
}

function Get-CcpUserSid {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try { return $identity.User } finally { $identity.Dispose() }
}

function Confirm-CcpPrivateStage {
    param([string]$Path)
    Confirm-CcpPath $Path
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { throw 'Private staging directory is missing.' }
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
        # The installer itself is Windows-only; allow isolated host fixtures to
        # exercise the same path checks with an owner-only Unix staging directory.
        if ([IO.File]::GetUnixFileMode($Path) -ne [IO.UnixFileMode]448) { throw 'Staging directory is not private.' }
        return
    }
    $sid = Get-CcpUserSid
    $acl = Get-Acl -LiteralPath $Path
    $rules = @($acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]))
    if (-not $acl.AreAccessRulesProtected -or $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -ne $sid.Value -or $rules.Count -ne 1) {
        throw 'Staging ACL must be owned by and limited to the current user.'
    }
    $rule = $rules[0]
    $inherit = [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
    if ($rule.IdentityReference.Value -ne $sid.Value -or $rule.IsInherited -or
        $rule.AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow -or
        $rule.FileSystemRights -ne [Security.AccessControl.FileSystemRights]::FullControl -or
        $rule.InheritanceFlags -ne $inherit -or $rule.PropagationFlags -ne [Security.AccessControl.PropagationFlags]::None) {
        throw 'Staging ACL must grant only the current user full control.'
    }
}

function New-CcpPrivateStage {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$Parent)
    if (-not $PSCmdlet.ShouldProcess($Parent, 'Create private CCP staging directory')) { return }
    Confirm-CcpPath $Parent
    $path = Join-Path $Parent ".ccp-stage-$([Guid]::NewGuid().ToString('N'))"
    if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
        $sid = Get-CcpUserSid
        $acl = [Security.AccessControl.DirectorySecurity]::new()
        $acl.SetOwner($sid)
        $acl.SetAccessRuleProtection($true, $false)
        $rule = [Security.AccessControl.FileSystemAccessRule]::new($sid,
            [Security.AccessControl.FileSystemRights]::FullControl,
            [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
            [Security.AccessControl.PropagationFlags]::None,
            [Security.AccessControl.AccessControlType]::Allow)
        $acl.AddAccessRule($rule)
        $directory = [IO.DirectoryInfo]::new($path)
        if ($PSVersionTable.PSEdition -eq 'Desktop') { $directory.Create($acl) }
        else {
            Add-Type -AssemblyName System.IO.FileSystem.AccessControl
            [IO.FileSystemAclExtensions]::Create($directory, $acl)
        }
    } else {
        [IO.Directory]::CreateDirectory($path, [IO.UnixFileMode]448) | Out-Null
    }
    Confirm-CcpPrivateStage $path
    return $path
}

function Remove-CcpPrivateStage {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$Path)
    if (-not $PSCmdlet.ShouldProcess($Path, 'Remove private CCP staging directory')) { return }
    Confirm-CcpPrivateStage $Path
    Remove-Item -LiteralPath $Path -Recurse -Force
}

function New-CcpDirectory {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$Path)
    if (-not $PSCmdlet.ShouldProcess($Path, 'Create CCP directory')) { return }
    Confirm-CcpPath $Path -AllowMissing
    [IO.Directory]::CreateDirectory($Path) | Out-Null
    Confirm-CcpPath $Path
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
    Confirm-CcpPrivateStage $Stage
    $base = "https://github.com/$CcpRepository/releases/download/$Tag"
    foreach ($name in @('ccp-release.json', 'ccp-provenance.json', $CcpClientAsset,
            'ccp-mcp.tar.gz', 'build-info.json')) {
        Save-CcpReleaseFile -Uri "$base/$name" -OutFile (Join-Path $Stage $name)
    }
}

function Confirm-CcpRelease {
    param([string]$Stage, [string]$ReleaseChannel, [string]$Tag)
    Confirm-CcpPrivateStage $Stage
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
    Confirm-CcpPath $Destination -AllowMissing
    Confirm-CcpPath $Source
    if (Test-Path -LiteralPath $Destination -PathType Container) { throw 'Refusing to replace a directory.' }
    $stage = New-CcpPrivateStage (Split-Path -Parent $Destination)
    try {
        $payload = Join-Path $stage 'payload'
        [IO.File]::Copy($Source, $payload, $false)
        # Recheck both names and the staging ACL immediately before publication.
        Confirm-CcpPath $payload
        Confirm-CcpPath $Destination -AllowMissing
        Confirm-CcpPrivateStage $stage
        if ([IO.File]::Exists($Destination)) {
            [IO.File]::Replace($payload, $Destination, [NullString]::Value)
        } else {
            [IO.File]::Move($payload, $Destination)
        }
    } finally { Remove-CcpPrivateStage $stage }
}

function Install-CcpCommand {
    param([string]$Source, [string]$BinDirectory)
    $client = Join-Path $BinDirectory 'ccp-client.exe'
    $alias = Join-Path $BinDirectory 'ccp.exe'
    Confirm-CcpPath $client -AllowMissing
    Confirm-CcpPath $alias -AllowMissing
    # Refresh only our own alias: its bytes must match the previous client.
    $owned = (Test-Path -LiteralPath $alias -PathType Leaf) -and
        (Test-Path -LiteralPath $client -PathType Leaf) -and
        ((Get-FileHash -LiteralPath $alias).Hash -eq (Get-FileHash -LiteralPath $client).Hash)
    $existing = Get-Command ccp -ErrorAction SilentlyContinue | Select-Object -First 1
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
    param([string]$Stage, [string]$ProfileDirectory, [string]$BootstrapPython)
    $venv = Join-Path $ProfileDirectory '.ccp-mcp\venv'
    Confirm-CcpPath $venv -AllowMissing
    $python = Join-Path $venv 'Scripts\python.exe'
    Confirm-CcpPath $python -AllowMissing
    if (-not (Test-Path -LiteralPath $python)) {
        Invoke-CcpNative $BootstrapPython @('-m', 'venv', $venv)
    }
    Confirm-CcpPath $python
    Invoke-CcpNative $python @('-m', 'pip', 'install', '--quiet', '--upgrade',
        '--force-reinstall', (Join-Path $Stage 'ccp-mcp.tar.gz'))
    $command = Join-Path $venv 'Scripts\ccp-mcp-server.exe'
    Confirm-CcpPath $command
    if (-not (Test-Path -LiteralPath $command -PathType Leaf)) { throw 'MCP entry point is missing.' }
    return $command
}

function Backup-CcpConfig {
    param([string]$Path)
    Confirm-CcpPath $Path -AllowMissing
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $stamp = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ')
    $backup = "$Path.backup.$stamp.$([Guid]::NewGuid().ToString('N'))"
    Confirm-CcpPath $backup -AllowMissing
    [IO.File]::Copy($Path, $backup, $false)
}

function Set-CcpCodexConfig {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$Path, [string]$McpCommand, [string]$Client)
    if (-not $PSCmdlet.ShouldProcess($Path, 'Configure CCP')) { return }
    Confirm-CcpPath $Path
    $stage = New-CcpPrivateStage (Split-Path -Parent $Path)
    $savedHome = $env:CODEX_HOME
    try {
        $stagedConfig = Join-Path $stage 'config.toml'
        [IO.File]::Copy($Path, $stagedConfig, $false)
        $env:CODEX_HOME = $stage
        Invoke-CcpNative 'codex' @('mcp', 'add', 'ccp', '--env', "CCP_CLIENT_BIN=$Client", '--', $McpCommand)
        Backup-CcpConfig $Path
        Publish-CcpFile $stagedConfig $Path
    } finally {
        [Environment]::SetEnvironmentVariable('CODEX_HOME', $savedHome, 'Process')
        Remove-CcpPrivateStage $stage
    }
}

function Set-CcpJsonHost {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$Path, [string]$McpCommand, [string]$Client)
    if (-not $PSCmdlet.ShouldProcess($Path, 'Configure CCP')) { return }
    Confirm-CcpPath $Path -AllowMissing
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
    New-CcpDirectory $directory
    $stageDirectory = New-CcpPrivateStage $directory
    $stage = Join-Path $stageDirectory 'config.json'
    try {
        $json = $config | ConvertTo-Json -Depth 100 -WarningAction Stop
        [IO.File]::WriteAllText($stage, $json, [Text.UTF8Encoding]::new($false))
        Backup-CcpConfig $Path
        Publish-CcpFile $stage $Path
    } finally {
        Remove-CcpPrivateStage $stageDirectory
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
    Confirm-CcpPath $codexConfig -AllowMissing
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
        Set-CcpCodexConfig $codexConfig $McpCommand $Client
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
    Resolve-CcpApplication 'gh' | Out-Null
    $python = Resolve-CcpPython
    $tag = Get-CcpReleaseTag $ReleaseChannel $RequestedVersion
    $bin = [IO.Path]::GetFullPath($BinDirectory)
    Confirm-CcpPath $bin -AllowMissing
    $stage = New-CcpPrivateStage $env:USERPROFILE
    try {
        Write-Information -InformationAction Continue 'Downloading and verifying the release; no software is installed until verification succeeds.'
        Get-CcpReleaseFile $tag $stage
        Confirm-CcpRelease $stage $ReleaseChannel $tag
        New-CcpDirectory $bin
        Install-CcpCommand (Join-Path $stage $CcpClientAsset) $bin
        Set-CcpUserPath $bin
        Write-Information -InformationAction Continue "Installed verified Windows client from $tag."
        Write-Information -InformationAction Continue 'Installing the matching MCP bridge.'
        $mcp = Install-CcpMcp $stage $env:USERPROFILE $python
        Set-CcpMcpHost $env:USERPROFILE $mcp (Join-Path $bin 'ccp-client.exe')
        Write-Information -InformationAction Continue 'Installation complete. Open a new terminal and restart MCP hosts. Re-run the same command to update.'
    } finally {
        Remove-CcpPrivateStage $stage
    }
}

try { Install-CcpWindows $Channel $Version $InstallDir }
catch {
    $prerequisite = $_.Exception.Data['CcpPrerequisite']
    if ($prerequisite) { throw "CCP installation failed: $prerequisite" }
    throw 'CCP installation failed. Required tools, verified release assets, and writable user directories are needed. No unverified software was installed.'
}
