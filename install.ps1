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
$script:CcpUpgrade = $null

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
    param([string]$Executable, [string[]]$ArgumentList, [switch]$Capture)
    # Resolve first so a missing command cannot reuse a previous exit code.
    $tool = Resolve-CcpApplication $Executable
    # Windows PowerShell can turn native stderr into terminating errors even
    # on success. Suppress tool output (which may contain private details) and
    # use the exit code for success, with a function-local error preference.
    $ErrorActionPreference = 'Continue'
    # Disable PowerShell 7's native-error conversion locally; the explicit
    # exit-code check below always throws on failure, whatever the caller chose.
    $PSNativeCommandUseErrorActionPreference = $false
    $encoding = $null
    try {
        if ($Capture) {
            $encoding = [Console]::OutputEncoding
            [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
        }
        $output = @(& $tool @ArgumentList 2>$null)
        $exitCode = $LASTEXITCODE
        if ($exitCode -ne 0) { throw 'A required external command failed.' }
    } finally { if ($null -ne $encoding) { [Console]::OutputEncoding = $encoding } }
    if ($Capture) { return $output -join [Environment]::NewLine }
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
    $manifest = Get-Content -Encoding UTF8 -LiteralPath $manifestPath -Raw | ConvertFrom-Json
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

function Get-CcpFileDigest {
    param([string]$Path)
    Confirm-CcpPath $Path -AllowMissing
    if (Test-Path -LiteralPath $Path -PathType Container) { throw 'Upgrade destination is a directory.' }
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-CcpUpgradeTarget {
    param([string]$BinDirectory)
    return @{
        client = (Join-Path $BinDirectory 'ccp-client.exe')
        alias = (Join-Path $BinDirectory 'ccp.exe')
        install = (Join-Path $BinDirectory '.ccp-install.json')
        claude = (Join-Path $env:USERPROFILE '.claude.json')
        cursor = (Join-Path $env:USERPROFILE '.cursor\mcp.json')
        codex = (Join-Path $env:USERPROFILE '.codex\config.toml')
    }
}

function Save-CcpUpgradeState {
    param($State)
    Confirm-CcpPrivateStage $State.directory
    $temporary = Join-Path $State.directory "state-$([Guid]::NewGuid().ToString('N')).json"
    $path = Join-Path $State.directory 'state.json'
    try {
        [IO.File]::WriteAllText($temporary, ($State | ConvertTo-Json -Depth 20 -WarningAction Stop), [Text.UTF8Encoding]::new($false))
        Confirm-CcpPath $path -AllowMissing
        Confirm-CcpPrivateStage $State.directory
        if ([IO.File]::Exists($path)) { [IO.File]::Replace($temporary, $path, [NullString]::Value) }
        else { [IO.File]::Move($temporary, $path) }
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}

function Start-CcpUpgrade {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$BinDirectory, [string]$Candidate)
    if (-not $PSCmdlet.ShouldProcess($BinDirectory, 'Stage recoverable CCP upgrade')) { return }
    $directory = New-CcpPrivateStage $BinDirectory
    $journal = Join-Path $BinDirectory '.ccp-upgrade.json'
    try {
        $files = @()
        $targets = Get-CcpUpgradeTarget $BinDirectory
        foreach ($key in @('client', 'alias', 'install', 'claude', 'cursor', 'codex')) {
            $hash = Get-CcpFileDigest $targets[$key]
            if ($hash) {
                [IO.File]::Copy($targets[$key], (Join-Path $directory $key), $false)
                if ((Get-CcpFileDigest (Join-Path $directory $key)) -ne $hash) { throw 'Upgrade input changed during backup.' }
            }
            $files += [PSCustomObject]@{ key = $key; existed = [bool]$hash; hash = $hash }
        }
        $state = [PSCustomObject]@{ format = 1; directory = $directory; candidate = $Candidate; files = $files; touched = @(); committed = $false }
        Save-CcpUpgradeState $state
        $pointer = Join-Path $directory 'journal.json'
        [IO.File]::WriteAllText($pointer, (@{format = 1; directory = $directory} | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
        Confirm-CcpPath $journal -AllowMissing
        [IO.File]::Move($pointer, $journal)
        return $state
    } catch {
        Remove-CcpPrivateStage $directory
        throw
    }
}

function Publish-CcpUpgradeFile {
    param([string]$Source, [string]$Destination)
    if ($script:CcpUpgrade) {
        $targets = Get-CcpUpgradeTarget (Split-Path -Parent $script:CcpUpgrade.directory)
        $key = @($targets.Keys | Where-Object { $targets[$_] -eq $Destination })
        if ($key.Count -ne 1) { throw 'Unrecognized upgrade destination.' }
        if ($script:CcpUpgrade.touched -notcontains $key[0]) {
            $original = $script:CcpUpgrade.files | Where-Object key -EQ $key[0]
            if ((Get-CcpFileDigest $Destination) -ne $original.hash) { throw 'Upgrade destination changed before publication.' }
            $script:CcpUpgrade.touched += $key[0]
            Save-CcpUpgradeState $script:CcpUpgrade
        }
    }
    Publish-CcpFile $Source $Destination
}

function Restore-CcpUpgrade {
    param([string]$BinDirectory)
    $journal = Join-Path $BinDirectory '.ccp-upgrade.json'
    Confirm-CcpPath $journal -AllowMissing
    if (-not (Test-Path -LiteralPath $journal)) { return }
    if ((Get-Item -LiteralPath $journal -Force).Length -gt 4096) { throw 'Invalid upgrade journal.' }
    $pointer = Get-Content -Encoding UTF8 -LiteralPath $journal -Raw | ConvertFrom-Json
    $directory = [IO.Path]::GetFullPath($pointer.directory)
    if ($pointer.format -ne 1 -or (Split-Path -Parent $directory) -ne $BinDirectory -or
        (Split-Path -Leaf $directory) -cnotmatch '^\.ccp-stage-[0-9a-f]{32}$') { throw 'Invalid upgrade journal directory.' }
    Confirm-CcpPrivateStage $directory
    $statePath = Join-Path $directory 'state.json'
    Confirm-CcpPath $statePath
    if ((Get-Item -LiteralPath $statePath -Force).Length -gt 65536) { throw 'Invalid upgrade state.' }
    $state = Get-Content -Encoding UTF8 -LiteralPath $statePath -Raw | ConvertFrom-Json
    $targets = Get-CcpUpgradeTarget $BinDirectory
    $keys = @($state.files | ForEach-Object key)
    if ($state.format -ne 1 -or $state.directory -ne $directory -or
        @($state.files).Count -ne $targets.Count -or @($keys | Select-Object -Unique).Count -ne $targets.Count) { throw 'Invalid upgrade snapshots.' }
    foreach ($file in $state.files) {
        if (-not $targets.ContainsKey($file.key) -or $file.existed -isnot [bool] -or
            ($file.existed -and $file.hash -cnotmatch '^[0-9a-f]{64}$') -or
            (-not $file.existed -and $null -ne $file.hash)) { throw 'Invalid upgrade snapshot.' }
    }
    if ($state.committed -isnot [bool] -or @($state.touched | Where-Object { -not $targets.ContainsKey($_) }).Count) { throw 'Invalid upgrade publication state.' }
    if ($state.candidate) {
        $parent = Join-Path $env:USERPROFILE '.ccp-mcp\generations'
        if ((Split-Path -Parent $state.candidate) -ne $parent -or
            (Split-Path -Leaf $state.candidate) -cnotmatch '^\.ccp-stage-[0-9a-f]{32}$') { throw 'Invalid upgrade candidate directory.' }
        Confirm-CcpPrivateStage $state.candidate
    }
    if (-not $state.committed) {
        $touched = @($state.touched)
        [array]::Reverse($touched)
        foreach ($key in $touched) {
            $file = $state.files | Where-Object key -EQ $key
            $current = Get-CcpFileDigest $targets[$key]
            if ($file.existed) {
                $backup = Join-Path $directory $key
                if ((Get-CcpFileDigest $backup) -ne $file.hash) { throw 'Upgrade backup changed.' }
                if ($current -ne $file.hash) { Publish-CcpFile $backup $targets[$key] }
            } elseif ($current) { [IO.File]::Delete($targets[$key]) }
        }
        if ($state.candidate) {
            Remove-CcpPrivateStage $state.candidate
        }
    }
    [IO.File]::Delete($journal)
    Remove-CcpPrivateStage $directory
}

function Get-CcpInstallState {
    param([string]$BinDirectory)
    $path = Join-Path $BinDirectory '.ccp-install.json'
    Confirm-CcpPath $path -AllowMissing
    if (-not (Test-Path -LiteralPath $path)) { return $null }
    if ((Get-Item -LiteralPath $path -Force).Length -gt 65536) { throw 'Invalid installation ownership record.' }
    $state = Get-Content -Encoding UTF8 -LiteralPath $path -Raw | ConvertFrom-Json
    if ($state.format -ne 1 -or $state.client_sha256 -cnotmatch '^[0-9a-f]{64}$' -or
        ($null -ne $state.alias_sha256 -and $state.alias_sha256 -cnotmatch '^[0-9a-f]{64}$')) { throw 'Invalid installation ownership record.' }
    Confirm-CcpPath $state.mcp_command
    if ($state.PSObject.Properties['mcp_client']) { Confirm-CcpPath $state.mcp_client }
    return $state
}

function Install-CcpCommand {
    param([string]$Source, [string]$BinDirectory)
    if (-not $script:CcpUpgrade) { throw 'Executable publication requires an upgrade transaction.' }
    $client = Join-Path $BinDirectory 'ccp-client.exe'
    $alias = Join-Path $BinDirectory 'ccp.exe'
    $hash = Get-CcpFileDigest $Source
    $aliasHash = Get-CcpFileDigest $alias
    $state = Get-CcpInstallState $BinDirectory
    # Adopt only a recorded alias or bytes matching this verified release.
    $owned = $aliasHash -and (($state -and $state.alias_sha256 -eq $aliasHash) -or $aliasHash -eq $hash)
    $existing = Get-Command ccp -ErrorAction SilentlyContinue | Select-Object -First 1
    $available = -not $aliasHash -and -not $existing
    $directory = New-CcpPrivateStage $BinDirectory
    try {
        $stagedClient = Join-Path $directory 'ccp-client.exe'
        $stagedAlias = Join-Path $directory 'ccp.exe'
        [IO.File]::Copy($Source, $stagedClient, $false)
        [IO.File]::Copy($Source, $stagedAlias, $false)
        Publish-CcpUpgradeFile $stagedClient $client
        if ($owned -or $available) {
            Publish-CcpUpgradeFile $stagedAlias $alias
            Write-Information -InformationAction Continue 'Installed ccp and ccp-client.'
        } else { Write-Warning 'An existing ccp command was retained. Use ccp-client.' }
    } finally { Remove-CcpPrivateStage $directory }
    return [bool]($owned -or $available)
}

function Invoke-CcpUpgrade {
    param([string]$Stage, [string]$BinDirectory, [string]$BootstrapPython)
    $previous = Get-CcpInstallState $BinDirectory
    $oldCommand = if ($previous) { $previous.mcp_command } else { Join-Path $env:USERPROFILE '.ccp-mcp\venv\Scripts\ccp-mcp-server.exe' }
    $oldClient = if ($previous -and $previous.PSObject.Properties['mcp_client']) { $previous.mcp_client } else { Join-Path $BinDirectory 'ccp-client.exe' }
    $candidate = Install-CcpMcp $Stage $env:USERPROFILE $BootstrapPython
    $state = $null
    try {
        $state = Start-CcpUpgrade $BinDirectory $candidate.directory
        $script:CcpUpgrade = $state
        $managedAlias = Install-CcpCommand (Join-Path $Stage $CcpClientAsset) $BinDirectory
        Set-CcpMcpHost $env:USERPROFILE $candidate.command $candidate.client $oldCommand $oldClient
        $hash = Get-CcpFileDigest (Join-Path $Stage $CcpClientAsset)
        $record = @{ format = 1; client_sha256 = $hash; alias_sha256 = $(if ($managedAlias) { $hash } else { $null }); mcp_command = $candidate.command; mcp_client = $candidate.client }
        $recordPath = Join-Path $state.directory 'install.json'
        [IO.File]::WriteAllText($recordPath, ($record | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
        Publish-CcpUpgradeFile $recordPath (Join-Path $BinDirectory '.ccp-install.json')
        $state.committed = $true
        Save-CcpUpgradeState $state
    } catch {
        $script:CcpUpgrade = $null
        try {
            if ($state) { Restore-CcpUpgrade $BinDirectory }
            else { Remove-CcpPrivateStage $candidate.directory }
        } catch {
            $failure = [InvalidOperationException]::new('Upgrade rollback is pending. Close CCP processes and re-run this installer to recover the previous install.')
            $failure.Data['CcpPrerequisite'] = $failure.Message
            throw $failure
        }
        throw
    } finally { $script:CcpUpgrade = $null }
    try { Restore-CcpUpgrade $BinDirectory }
    catch { Write-Warning 'Upgrade completed; private journal cleanup is pending and will be retried on the next run.' }
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
    $parent = Join-Path $ProfileDirectory '.ccp-mcp\generations'
    New-CcpDirectory $parent
    $directory = New-CcpPrivateStage $parent
    try {
        $venv = Join-Path $directory 'venv'
        Invoke-CcpNative $BootstrapPython @('-I', '-m', 'venv', $venv)
        $python = Join-Path $venv 'Scripts\python.exe'
        Confirm-CcpPath $python
        Invoke-CcpNative $python @('-I', '-m', 'pip', 'install', '--quiet', '--upgrade',
            (Join-Path $Stage 'ccp-mcp.tar.gz'))
        $version = (Get-Content -Encoding UTF8 -LiteralPath (Join-Path $Stage 'ccp-release.json') -Raw | ConvertFrom-Json).package_version
        $check = "import importlib.metadata as m, sys; import ccp_mcp_server.server; assert m.version('ccp-mcp-server') == sys.argv[1]; ep = next(e for e in m.distribution('ccp-mcp-server').entry_points if e.group == 'console_scripts' and e.name == 'ccp-mcp-server'); assert ep.value == 'ccp_mcp_server.server:main' and callable(ep.load())"
        Invoke-CcpNative $python @('-I', '-c', $check, $version)
        $command = Join-Path $venv 'Scripts\ccp-mcp-server.exe'
        Confirm-CcpPath $command
        if (-not (Test-Path -LiteralPath $command -PathType Leaf)) { throw 'MCP entry point is missing.' }
        $client = Join-Path $directory 'ccp-client.exe'
        [IO.File]::Copy((Join-Path $Stage $CcpClientAsset), $client, $false)
        if ((Get-CcpFileDigest $client) -ne (Get-CcpFileDigest (Join-Path $Stage $CcpClientAsset))) { throw 'Candidate client changed.' }
        Confirm-CcpPrivateStage $directory
        return [PSCustomObject]@{directory = $directory; command = $command; client = $client}
    } catch { Remove-CcpPrivateStage $directory; throw }
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
    param([string]$Path, [string]$McpCommand, [string]$Client, [string]$PreviousMcp = '', [string]$PreviousClient = '')
    if (-not $PSCmdlet.ShouldProcess($Path, 'Configure CCP')) { return }
    Confirm-CcpPath $Path
    $stage = New-CcpPrivateStage (Split-Path -Parent $Path)
    $savedHome = $env:CODEX_HOME
    try {
        $stagedConfig = Join-Path $stage 'config.toml'
        [IO.File]::Copy($Path, $stagedConfig, $false)
        $env:CODEX_HOME = $stage
        if ((Get-Content -Encoding UTF8 -LiteralPath $Path -Raw) -match '(?m)^\s*\[mcp_servers\.ccp(?:\]|\.)') {
            $current = Invoke-CcpNative 'codex' @('mcp', 'get', 'ccp', '--json') -Capture | ConvertFrom-Json
            $custom = $current.PSObject.Properties['enabled'] -and -not $current.enabled
            foreach ($property in $current.PSObject.Properties) {
                if ($property.Name -notin @('name', 'enabled', 'transport') -and $null -ne $property.Value) { $custom = $true }
            }
            foreach ($property in $current.transport.PSObject.Properties) {
                if ($property.Name -notin @('type', 'command', 'args', 'env') -and $null -ne $property.Value -and @($property.Value).Count) { $custom = $true }
            }
            if ($custom -or -not $PreviousMcp -or $current.transport.type -ne 'stdio' -or
                $current.transport.command -ne $PreviousMcp -or -not $current.transport.env -or
                -not $current.transport.env.PSObject.Properties['CCP_CLIENT_BIN'] -or
                $current.transport.env.CCP_CLIENT_BIN -ne $PreviousClient -or
                @($current.transport.args).Count -ne 0 -or @($current.transport.env.PSObject.Properties).Count -ne 1) {
                Write-Warning 'Existing Codex CCP MCP configuration retained; check its paths.'
                return
            }
        }
        Invoke-CcpNative 'codex' @('mcp', 'add', 'ccp', '--env', "CCP_CLIENT_BIN=$Client", '--', $McpCommand)
        Backup-CcpConfig $Path
        Publish-CcpUpgradeFile $stagedConfig $Path
    } finally {
        [Environment]::SetEnvironmentVariable('CODEX_HOME', $savedHome, 'Process')
        Remove-CcpPrivateStage $stage
    }
}

function Set-CcpJsonHost {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$Path, [string]$McpCommand, [string]$Client, [string]$PreviousMcp = '', [string]$PreviousClient = '')
    if (-not $PSCmdlet.ShouldProcess($Path, 'Configure CCP')) { return }
    Confirm-CcpPath $Path -AllowMissing
    $config = [PSCustomObject]@{}
    if (Test-Path -LiteralPath $Path) {
        $config = Get-Content -Encoding UTF8 -LiteralPath $Path -Raw | ConvertFrom-Json
    }
    if (-not $config.PSObject.Properties['mcpServers']) {
        $config | Add-Member -NotePropertyName mcpServers -NotePropertyValue ([PSCustomObject]@{})
    }
    if ($config.mcpServers.PSObject.Properties['ccp']) {
        $current = $config.mcpServers.ccp
        if (-not $PreviousMcp -or $current.command -ne $PreviousMcp -or -not $current.PSObject.Properties['env'] -or
            -not $current.env -or -not $current.env.PSObject.Properties['CCP_CLIENT_BIN'] -or $current.env.CCP_CLIENT_BIN -ne $PreviousClient) {
            Write-Warning 'Existing CCP MCP configuration retained; check its command and CCP_CLIENT_BIN.'
            return
        }
        $current.command = $McpCommand
        $current.env.CCP_CLIENT_BIN = $Client
    } else {
        $config.mcpServers | Add-Member -NotePropertyName ccp -NotePropertyValue @{
            command = $McpCommand; env = @{ CCP_CLIENT_BIN = $Client }
        }
    }
    $directory = Split-Path -Parent $Path
    New-CcpDirectory $directory
    $stageDirectory = New-CcpPrivateStage $directory
    $stage = Join-Path $stageDirectory 'config.json'
    try {
        $json = $config | ConvertTo-Json -Depth 100 -WarningAction Stop
        [IO.File]::WriteAllText($stage, $json, [Text.UTF8Encoding]::new($false))
        Backup-CcpConfig $Path
        Publish-CcpUpgradeFile $stage $Path
    } finally {
        Remove-CcpPrivateStage $stageDirectory
    }
}

function Set-CcpMcpHost {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$ProfileDirectory, [string]$McpCommand, [string]$Client, [string]$PreviousMcp = '', [string]$PreviousClient = '')
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
        try { Set-CcpJsonHost $path $McpCommand $Client $PreviousMcp $PreviousClient; Write-Information -InformationAction Continue "$name MCP configuration checked." }
        catch {
            if ($script:CcpUpgrade) { throw 'MCP host configuration update failed.' }
            Write-Warning "$name MCP not configured: configuration could not be updated."
        }
    }
    $codexConfig = Join-Path $ProfileDirectory '.codex\config.toml'
    Confirm-CcpPath $codexConfig -AllowMissing
    if (-not (Test-Path -LiteralPath $codexConfig)) {
        Write-Warning 'Codex MCP not configured: no host configuration found.'
        return
    }
    try {
        # Let the host parse and write TOML; never interpolate Windows paths into it.
        Set-CcpCodexConfig $codexConfig $McpCommand $Client $PreviousMcp $PreviousClient
        Write-Information -InformationAction Continue 'Codex MCP configured.'
    } catch {
        if ($script:CcpUpgrade) { throw 'Codex MCP configuration update failed.' }
        Write-Warning 'Codex MCP not configured: codex mcp add failed or is unavailable.'
    }
}

function Install-CcpWindows {
    param([string]$ReleaseChannel, [string]$RequestedVersion, [string]$BinDirectory)
    if ($env:OS -ne 'Windows_NT' -or
        ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64' -and $env:PROCESSOR_ARCHITEW6432 -ne 'AMD64')) {
        throw 'This installer requires x86_64 Windows.'
    }
    if (-not $env:USERPROFILE -or -not $env:LOCALAPPDATA) { throw 'A Windows user profile is required.' }
    $bin = [IO.Path]::GetFullPath($BinDirectory)
    Confirm-CcpPath $bin -AllowMissing
    $lock = $null
    if (Test-Path -LiteralPath $bin) {
        $lockPath = Join-Path $bin '.ccp-upgrade.lock'
        Confirm-CcpPath $lockPath -AllowMissing
        $lock = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    }
    try {
        if ($lock) { Restore-CcpUpgrade $bin }
        Resolve-CcpApplication 'gh' | Out-Null
        $python = Resolve-CcpPython
        $tag = Get-CcpReleaseTag $ReleaseChannel $RequestedVersion
        $stage = New-CcpPrivateStage $env:USERPROFILE
        try {
            Write-Information -InformationAction Continue 'Downloading and verifying the release; no software is installed until verification succeeds.'
            Get-CcpReleaseFile $tag $stage
            Confirm-CcpRelease $stage $ReleaseChannel $tag
            New-CcpDirectory $bin
            if (-not $lock) {
                $lockPath = Join-Path $bin '.ccp-upgrade.lock'
                Confirm-CcpPath $lockPath -AllowMissing
                $lock = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
                Restore-CcpUpgrade $bin
            }
            Set-CcpUserPath $bin
            Write-Information -InformationAction Continue 'Building and validating the matching MCP bridge before replacing any commands.'
            Invoke-CcpUpgrade $stage $bin $python
            Write-Information -InformationAction Continue "Installed verified Windows client and MCP bridge from $tag."
            Write-Information -InformationAction Continue 'Installation complete. Open a new terminal and restart MCP hosts. Re-run the same command to update.'
        } finally {
            Remove-CcpPrivateStage $stage
        }
    } finally { if ($lock) { $lock.Dispose() } }
}

try { Install-CcpWindows $Channel $Version $InstallDir }
catch {
    $prerequisite = $_.Exception.Data['CcpPrerequisite']
    if ($prerequisite) { throw "CCP installation failed: $prerequisite" }
    throw 'CCP installation failed. Required tools, verified release assets, and writable user directories are needed. No unverified software was installed.'
}
