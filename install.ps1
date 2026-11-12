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
$script:CcpRetentionDirectory = $null

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

function Remove-CcpTree {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$Path)
    if (-not $PSCmdlet.ShouldProcess($Path, 'Delete owned staging tree without following links')) { return }
    foreach ($child in [IO.Directory]::EnumerateFileSystemEntries($Path)) {
        $attributes = [IO.File]::GetAttributes($child)
        if ($attributes -band [IO.FileAttributes]::ReparsePoint) {
            if ($attributes -band [IO.FileAttributes]::Directory) { [IO.Directory]::Delete($child, $false) }
            else { [IO.File]::Delete($child) }
        } elseif ($attributes -band [IO.FileAttributes]::Directory) { Remove-CcpTree $child }
        else {
            [IO.File]::SetAttributes($child, ($attributes -band (-bnot [IO.FileAttributes]::ReadOnly)))
            [IO.File]::Delete($child)
        }
    }
    [IO.Directory]::Delete($Path, $false)
}

function Remove-CcpPrivateStage {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$Path)
    if (-not $PSCmdlet.ShouldProcess($Path, 'Remove private CCP staging directory')) { return }
    Confirm-CcpPrivateStage $Path
    Remove-CcpTree $Path
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
            $release.tag_name -cnotmatch '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {
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
            $files += [PSCustomObject]@{ key = $key; existed = [bool]$hash; hash = $hash; new_hash = $null }
        }
        $state = [PSCustomObject]@{ format = 2; directory = $directory; candidate = $Candidate; files = $files; touched = @(); committed = $false; cleanup_ready = $false; alias_disowned = $false; alias_record_hash = $null }
        Save-CcpUpgradeState $state
        $pointer = Join-Path $directory 'journal.json'
        [IO.File]::WriteAllText($pointer, (@{format = 2; directory = $directory; cleanup_done = $false} | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
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
        $file = $script:CcpUpgrade.files | Where-Object key -EQ $key[0]
        $current = Get-CcpFileDigest $Destination
        if ($current -ne $file.hash -and ($null -eq $file.new_hash -or $current -ne $file.new_hash)) {
            if ($key[0] -eq 'alias') {
                $script:CcpUpgrade.alias_disowned = $true
                Save-CcpUpgradeState $script:CcpUpgrade
                Write-Warning 'An unrelated replacement ccp.exe was retained; CCP alias ownership is dropped.'
            }
            throw 'Upgrade destination changed outside the installer; it was not overwritten.'
        }
        if ($script:CcpUpgrade.touched -notcontains $key[0]) { $script:CcpUpgrade.touched += $key[0] }
        $file.new_hash = Get-CcpFileDigest $Source
        Save-CcpUpgradeState $script:CcpUpgrade
    }
    Publish-CcpFile $Source $Destination
}

function Save-CcpCleanupPointer {
    param([string]$BinDirectory, [string]$Directory)
    Confirm-CcpPrivateStage $Directory
    $source = Join-Path $Directory 'cleanup.json'
    $journal = Join-Path $BinDirectory '.ccp-upgrade.json'
    [IO.File]::WriteAllText($source, (@{format = 2; directory = $Directory; cleanup_done = $true} | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    Confirm-CcpPath $journal
    [IO.File]::Replace($source, $journal, [NullString]::Value)
}

function Restore-CcpUpgradeFile {
    param($State, $File, [string]$Destination)
    $current = Get-CcpFileDigest $Destination
    if ($File.key -eq 'install' -and $State.alias_record_hash -and $current -eq $State.alias_record_hash) { return }
    if ($current -ne $File.hash -and ($null -eq $File.new_hash -or $current -ne $File.new_hash)) {
        if ($File.key -eq 'alias') {
            $State.alias_disowned = $true
            Save-CcpUpgradeState $State
            Write-Warning 'An unrelated replacement ccp.exe was retained; CCP alias ownership is dropped.'
            return
        }
        Write-Warning 'A recovery destination changed outside the installer and was left untouched; inspect it before retrying.'
        throw 'A recovery destination changed outside the installer; it was left untouched.'
    }
    if ($File.existed) {
        $backup = Join-Path $State.directory $File.key
        if ((Get-CcpFileDigest $backup) -ne $File.hash) { throw 'Upgrade backup changed.' }
        if ($current -ne $File.hash) { Publish-CcpFile $backup $Destination }
    } elseif ($current) { [IO.File]::Delete($Destination) }
}

function Clear-CcpRecoveredAlias {
    param($State, [string]$Destination)
    if (-not $State.alias_disowned) { return }
    $file = $State.files | Where-Object key -EQ 'install'
    if (-not $file.existed) { return }
    $record = Get-Content -Encoding UTF8 -LiteralPath (Join-Path $State.directory 'install') -Raw | ConvertFrom-Json
    $record.alias_sha256 = $null
    $source = Join-Path $State.directory 'disowned-install.json'
    [IO.File]::WriteAllText($source, ($record | ConvertTo-Json -Depth 20), [Text.UTF8Encoding]::new($false))
    $hash = Get-CcpFileDigest $source
    $current = Get-CcpFileDigest $Destination
    if ($current -ne $file.hash -and (-not $file.new_hash -or $current -ne $file.new_hash) -and
        (-not $State.alias_record_hash -or $current -ne $State.alias_record_hash)) { throw 'Installation ownership changed outside recovery.' }
    $State.alias_record_hash = $hash
    Save-CcpUpgradeState $State
    if ($current -ne $hash) { Publish-CcpFile $source $Destination }
}

function Complete-CcpUpgradeCleanup {
    param([string]$BinDirectory, [string]$Directory)
    # This pointer survives removal of state.json, so every deletion is restartable.
    Save-CcpCleanupPointer $BinDirectory $Directory
    Remove-CcpPrivateStage $Directory
    Remove-CcpUpgradeJournal $BinDirectory
}

function Remove-CcpUpgradeJournal {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$BinDirectory)
    if (-not $PSCmdlet.ShouldProcess($BinDirectory, 'Remove completed upgrade journal')) { return }
    $path = Join-Path $BinDirectory '.ccp-upgrade.json'
    Confirm-CcpPath $path -AllowMissing
    [IO.File]::Delete($path)
}

function Restore-CcpUpgrade {
    param([string]$BinDirectory)
    $journal = Join-Path $BinDirectory '.ccp-upgrade.json'
    Confirm-CcpPath $journal -AllowMissing
    if (-not (Test-Path -LiteralPath $journal)) { return }
    if ((Get-Item -LiteralPath $journal -Force).Length -gt 4096) { throw 'Invalid upgrade journal.' }
    $pointer = Get-Content -Encoding UTF8 -LiteralPath $journal -Raw | ConvertFrom-Json
    $directory = [IO.Path]::GetFullPath($pointer.directory)
    if ($pointer.format -ne 2 -or $pointer.cleanup_done -isnot [bool] -or
        (Split-Path -Parent $directory) -ne $BinDirectory -or
        (Split-Path -Leaf $directory) -cnotmatch '^\.ccp-stage-[0-9a-f]{32}$') { throw 'Invalid upgrade journal directory.' }
    Confirm-CcpPath $directory -AllowMissing
    if ($pointer.cleanup_done) {
        if (Test-Path -LiteralPath $directory) { Remove-CcpPrivateStage $directory }
        Remove-CcpUpgradeJournal $BinDirectory
        return
    }
    Confirm-CcpPrivateStage $directory
    $statePath = Join-Path $directory 'state.json'
    Confirm-CcpPath $statePath
    if ((Get-Item -LiteralPath $statePath -Force).Length -gt 65536) { throw 'Invalid upgrade state.' }
    $state = Get-Content -Encoding UTF8 -LiteralPath $statePath -Raw | ConvertFrom-Json
    $targets = Get-CcpUpgradeTarget $BinDirectory
    $keys = @($state.files | ForEach-Object key)
    if ($state.format -ne 2 -or $state.directory -ne $directory -or
        @($state.files).Count -ne $targets.Count -or @($keys | Select-Object -Unique).Count -ne $targets.Count) { throw 'Invalid upgrade snapshots.' }
    foreach ($file in $state.files) {
        if (-not $targets.ContainsKey($file.key) -or $file.existed -isnot [bool] -or
            ($file.existed -and $file.hash -cnotmatch '^[0-9a-f]{64}$') -or
            (-not $file.existed -and $null -ne $file.hash) -or
            ($null -ne $file.new_hash -and $file.new_hash -cnotmatch '^[0-9a-f]{64}$')) { throw 'Invalid upgrade snapshot.' }
    }
    if ($state.committed -isnot [bool] -or $state.cleanup_ready -isnot [bool] -or $state.alias_disowned -isnot [bool] -or
        ($state.alias_record_hash -and $state.alias_record_hash -cnotmatch '^[0-9a-f]{64}$') -or
        @($state.touched | Where-Object { -not $targets.ContainsKey($_) }).Count) { throw 'Invalid upgrade publication state.' }
    if ($state.candidate) {
        $parent = Join-Path $env:USERPROFILE '.ccp-mcp\generations'
        if ((Split-Path -Parent $state.candidate) -ne $parent -or
            (Split-Path -Leaf $state.candidate) -cnotmatch '^\.ccp-stage-[0-9a-f]{32}$') { throw 'Invalid upgrade candidate directory.' }
        Confirm-CcpPath $state.candidate -AllowMissing
        if (Test-Path -LiteralPath $state.candidate) { Confirm-CcpPrivateStage $state.candidate }
        elseif (-not $state.cleanup_ready -or $state.committed) { throw 'Upgrade candidate disappeared before cleanup.' }
    }
    if (-not $state.cleanup_ready) {
        if (-not $state.committed) {
            $touched = @($state.touched)
            [array]::Reverse($touched)
            foreach ($key in $touched) { Restore-CcpUpgradeFile $state ($state.files | Where-Object key -EQ $key) $targets[$key] }
            Clear-CcpRecoveredAlias $state $targets.install
        }
        $state.cleanup_ready = $true
        Save-CcpUpgradeState $state
    }
    if (-not $state.committed -and $state.candidate -and (Test-Path -LiteralPath $state.candidate)) { Remove-CcpPrivateStage $state.candidate }
    Complete-CcpUpgradeCleanup $BinDirectory $directory
}

function Invoke-CcpRecovery {
    param([string]$BinDirectory)
    try { Restore-CcpUpgrade $BinDirectory }
    catch {
        $failure = [InvalidOperationException]::new('Installation is mid-recovery. Close CCP and MCP processes, then re-run the same command to repair it. Recovery must finish before another upgrade.')
        $failure.Data['CcpPrerequisite'] = $failure.Message
        throw $failure
    }
}

function Get-CcpRetentionRoot {
    $context = Get-Variable CcpRetentionDirectory -Scope Script -ErrorAction SilentlyContinue
    if ($context) { return $context.Value }
}

function Get-CcpRetention {
    param([string]$BinDirectory)
    $path = Join-Path $BinDirectory '.ccp-retention.json'
    Confirm-CcpPath $path -AllowMissing
    if (-not (Test-Path -LiteralPath $path)) { return [PSCustomObject]@{format = 1; next_id = 0; generations = @(); backups = @()} }
    if ((Get-Item -LiteralPath $path -Force).Length -gt 65536) { throw 'Invalid retention record.' }
    $record = Get-Content -Encoding UTF8 -LiteralPath $path -Raw | ConvertFrom-Json
    if ($record.format -ne 1 -or ($record.next_id -isnot [int] -and $record.next_id -isnot [long]) -or $record.next_id -lt 0) { throw 'Invalid retention inventory.' }
    $ids = @()
    foreach ($entry in @($record.generations) + @($record.backups)) {
        if (($entry.id -isnot [int] -and $entry.id -isnot [long]) -or $entry.id -le 0 -or $entry.id -gt $record.next_id) { throw 'Invalid retention sequence.' }
        $ids += $entry.id
    }
    if (@($ids | Select-Object -Unique).Count -ne $ids.Count) { throw 'Duplicate retention ownership entry.' }
    return $record
}

function Save-CcpRetention {
    param($Record, [string]$BinDirectory)
    $directory = New-CcpPrivateStage $BinDirectory
    try {
        $source = Join-Path $directory 'retention.json'
        $destination = Join-Path $BinDirectory '.ccp-retention.json'
        [IO.File]::WriteAllText($source, ($Record | ConvertTo-Json -Depth 20 -WarningAction Stop), [Text.UTF8Encoding]::new($false))
        Confirm-CcpPath $destination -AllowMissing
        if ([IO.File]::Exists($destination)) { [IO.File]::Replace($source, $destination, [NullString]::Value) }
        else { [IO.File]::Move($source, $destination) }
    } finally { Remove-CcpPrivateStage $directory }
}

function Register-CcpGeneration {
    param([string]$Directory)
    $bin = Get-CcpRetentionRoot
    if (-not $bin) { throw 'Generation creation requires an installation context.' }
    Confirm-CcpPrivateStage $Directory
    $marker = Join-Path $Directory '.ccp-generation.json'
    [IO.File]::WriteAllText($marker, (@{format = 1; token = [Guid]::NewGuid().ToString('N'); install_root = $bin} | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    $record = Get-CcpRetention $bin
    if ($record.next_id -eq [long]::MaxValue) { throw 'Retention sequence is exhausted.' }
    $record.next_id++
    $record.generations += [PSCustomObject]@{
        id = $record.next_id; directory = $Directory; marker_hash = (Get-CcpFileDigest $marker)
        creation_ticks = (Get-Item -LiteralPath $Directory -Force).CreationTimeUtc.Ticks; status = 'candidate'
    }
    Save-CcpRetention $record $bin
}

function Register-CcpBackup {
    param([string]$Path, [string]$Backup, [string]$Hash)
    $bin = Get-CcpRetentionRoot
    if (-not $bin) { return }
    $targets = Get-CcpUpgradeTarget $bin
    $key = @(@('claude', 'cursor', 'codex') | Where-Object { $targets[$_] -eq $Path })
    if ($key.Count -ne 1) { throw 'Unrecognized configuration backup.' }
    $record = Get-CcpRetention $bin
    if ($record.next_id -eq [long]::MaxValue) { throw 'Retention sequence is exhausted.' }
    $record.next_id++
    $record.backups += [PSCustomObject]@{id = $record.next_id; config = $key[0]; path = $Backup; hash = $Hash}
    Save-CcpRetention $record $bin
}

function Remove-CcpRecordedBackup {
    [CmdletBinding(SupportsShouldProcess)]
    param($Record, [string]$BinDirectory)
    if (-not $PSCmdlet.ShouldProcess($BinDirectory, 'Prune recorded configuration backups')) { return }
    $targets = Get-CcpUpgradeTarget $BinDirectory
    $present = @()
    foreach ($entry in $Record.backups) {
        if ($entry.config -notin @('claude', 'cursor', 'codex') -or $entry.hash -cnotmatch '^[0-9a-f]{64}$') { throw 'Invalid recorded backup.' }
        $config = $targets[$entry.config]
        $pattern = '^' + [regex]::Escape((Split-Path -Leaf $config)) + '\.backup\.[0-9]{8}T[0-9]{9}Z\.[0-9a-f]{32}$'
        if ((Split-Path -Parent $entry.path) -ne (Split-Path -Parent $config) -or (Split-Path -Leaf $entry.path) -cnotmatch $pattern) { throw 'Invalid backup ownership path.' }
        $current = Get-CcpFileDigest $entry.path
        if (-not $current) { continue }
        if ($current -ne $entry.hash) { Write-Warning 'A modified backup was retained and is no longer installer-owned.'; continue }
        $present += $entry
    }
    $keep = @()
    foreach ($key in @('claude', 'cursor', 'codex')) {
        $ordered = @($present | Where-Object config -EQ $key | Sort-Object id -Descending)
        $keep += @($ordered | Select-Object -First 3)
        foreach ($entry in @($ordered | Select-Object -Skip 3)) {
            Confirm-CcpPath $entry.path
            if ((Get-CcpFileDigest $entry.path) -ne $entry.hash) { throw 'Backup changed before pruning.' }
            Remove-Item -LiteralPath $entry.path -Force
        }
    }
    $Record.backups = $keep
    Save-CcpRetention $Record $BinDirectory
}

function Remove-CcpRecordedGeneration {
    [CmdletBinding(SupportsShouldProcess)]
    param($Record, [string]$BinDirectory)
    if (-not $PSCmdlet.ShouldProcess($BinDirectory, 'Prune recorded MCP generations')) { return }
    $installed = Get-CcpInstallState $BinDirectory
    $active = if ($installed) { Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $installed.mcp_command)) } else { $null }
    $parent = Join-Path $env:USERPROFILE '.ccp-mcp\generations'
    if (-not $installed -and @($Record.generations | Where-Object status -EQ 'committed').Count) { throw 'Active installation ownership is unavailable; generations were retained.' }
    foreach ($entry in $Record.generations) {
        if ((Split-Path -Parent $entry.directory) -ne $parent -or
            (Split-Path -Leaf $entry.directory) -cnotmatch '^\.ccp-stage-[0-9a-f]{32}$' -or
            $entry.marker_hash -cnotmatch '^[0-9a-f]{64}$' -or $entry.status -notin @('candidate', 'committed', 'deleting')) { throw 'Invalid generation ownership record.' }
        if ($entry.directory -eq $active) { $entry.status = 'committed' }
    }
    $previous = $Record.generations | Where-Object { $_.directory -ne $active -and $_.status -eq 'committed' } | Sort-Object id -Descending | Select-Object -First 1
    $keep = @()
    foreach ($entry in $Record.generations) {
        Confirm-CcpPath $entry.directory -AllowMissing
        if (-not (Test-Path -LiteralPath $entry.directory)) { continue }
        if ($entry.directory -eq $active -or ($previous -and $entry.directory -eq $previous.directory)) { $keep += $entry; continue }
        Confirm-CcpPrivateStage $entry.directory
        $identity = (Get-Item -LiteralPath $entry.directory -Force).CreationTimeUtc.Ticks
        $marker = Join-Path $entry.directory '.ccp-generation.json'
        $markerHash = Get-CcpFileDigest $marker
        if ($identity -ne $entry.creation_ticks -or
            ($markerHash -and $markerHash -ne $entry.marker_hash) -or
            (-not $markerHash -and $entry.status -ne 'deleting')) {
            Write-Warning 'An unrecognized generation directory was retained and is no longer installer-owned.'
            continue
        }
        if ($entry.status -ne 'deleting') {
            $entry.status = 'deleting'
            Save-CcpRetention $Record $BinDirectory
        }
        Remove-CcpPrivateStage $entry.directory
    }
    $Record.generations = $keep
    Save-CcpRetention $Record $BinDirectory
}

function Invoke-CcpRetention {
    param([string]$BinDirectory)
    if (Test-Path -LiteralPath (Join-Path $BinDirectory '.ccp-upgrade.json')) { return }
    if (-not (Test-Path -LiteralPath (Join-Path $BinDirectory '.ccp-retention.json'))) { return }
    try {
        $record = Get-CcpRetention $BinDirectory
        Remove-CcpRecordedBackup $record $BinDirectory
        Remove-CcpRecordedGeneration $record $BinDirectory
    } catch {
        $failure = [InvalidOperationException]::new('Recorded cleanup is pending. Close old MCP processes and re-run the same command before another upgrade.')
        $failure.Data['CcpPrerequisite'] = $failure.Message
        throw $failure
    }
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
    $aliasHash = Get-CcpFileDigest $alias
    $state = Get-CcpInstallState $BinDirectory
    # Existing files require a recorded ownership hash; names or matching releases are insufficient.
    $owned = $aliasHash -and $state -and $state.alias_sha256 -eq $aliasHash
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
            if ($state) { Invoke-CcpRecovery $BinDirectory }
            else { Remove-CcpPrivateStage $candidate.directory }
        } catch {
            $failure = [InvalidOperationException]::new('Installation is mid-recovery. Close CCP and MCP processes, then re-run the same command to repair it. Recovery must finish before another upgrade.')
            $failure.Data['CcpPrerequisite'] = $failure.Message
            throw $failure
        }
        throw
    } finally { $script:CcpUpgrade = $null }
    try { Restore-CcpUpgrade $BinDirectory; Invoke-CcpRetention $BinDirectory }
    catch { Write-Warning 'Upgrade completed; recorded cleanup is pending. Close old MCP processes and re-run the same command before another upgrade.' }
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
        Register-CcpGeneration $directory
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
    $hash = Get-CcpFileDigest $Path
    Register-CcpBackup $Path $backup $hash
    [IO.File]::Copy($Path, $backup, $false)
    if ((Get-CcpFileDigest $backup) -ne $hash) { throw 'Configuration changed during backup.' }
    $bin = Get-CcpRetentionRoot
    if ($bin) { Remove-CcpRecordedBackup (Get-CcpRetention $bin) $bin }
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

function ConvertFrom-CcpConfigToken {
    param($Token)
    switch ($Token.get_Type().ToString()) {
        'Object' {
            $properties = [ordered]@{}
            foreach ($property in $Token.Properties()) { $properties.Add($property.Name, (ConvertFrom-CcpConfigToken $property.Value)) }
            return [PSCustomObject]$properties
        }
        'Array' {
            $values = [Collections.Generic.List[object]]::new()
            foreach ($child in $Token.Children()) { $values.Add((ConvertFrom-CcpConfigToken $child)) }
            return ,$values.ToArray()
        }
        default { return $Token.get_Value() }
    }
}

function ConvertFrom-CcpConfigJson {
    param([string]$Json)
    # Windows PowerShell keeps ISO strings; Core introduced timestamp coercion.
    if ($PSVersionTable.PSVersion.Major -lt 6) { return ($Json | ConvertFrom-Json) }
    if ((Get-Command ConvertFrom-Json).Parameters.ContainsKey('DateKind')) {
        return ($Json | ConvertFrom-Json -DateKind String)
    }
    # Older Core versions already bundle Json.NET, but lack -DateKind String.
    $settings = [Newtonsoft.Json.JsonSerializerSettings]::new()
    $settings.DateParseHandling = [Newtonsoft.Json.DateParseHandling]::None
    $settings.MaxDepth = 100
    $token = [Newtonsoft.Json.JsonConvert]::DeserializeObject($Json, $settings)
    return (ConvertFrom-CcpConfigToken $token)
}

function Set-CcpJsonHost {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$Path, [string]$McpCommand, [string]$Client, [string]$PreviousMcp = '', [string]$PreviousClient = '')
    if (-not $PSCmdlet.ShouldProcess($Path, 'Configure CCP')) { return }
    Confirm-CcpPath $Path -AllowMissing
    $config = [PSCustomObject]@{}
    if (Test-Path -LiteralPath $Path) {
        $config = ConvertFrom-CcpConfigJson (Get-Content -Encoding UTF8 -LiteralPath $Path -Raw)
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
        if ($lock) {
            $script:CcpRetentionDirectory = $bin
            Invoke-CcpRecovery $bin
            Invoke-CcpRetention $bin
        }
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
                $script:CcpRetentionDirectory = $bin
                Invoke-CcpRecovery $bin
                Invoke-CcpRetention $bin
            }
            Set-CcpUserPath $bin
            Write-Information -InformationAction Continue 'Building and validating the matching MCP bridge before replacing any commands.'
            Invoke-CcpUpgrade $stage $bin $python
            Write-Information -InformationAction Continue "Installed verified Windows client and MCP bridge from $tag."
            Write-Information -InformationAction Continue 'Installation complete. Open a new terminal and restart MCP hosts. Re-run the same command to update.'
        } finally {
            Remove-CcpPrivateStage $stage
        }
    } finally {
        $script:CcpRetentionDirectory = $null
        if ($lock) { $lock.Dispose() }
    }
}

try { Install-CcpWindows $Channel $Version $InstallDir }
catch {
    $prerequisite = $_.Exception.Data['CcpPrerequisite']
    if ($prerequisite) { throw "CCP installation failed: $prerequisite" }
    throw 'CCP installation failed. Required tools, verified release assets, and writable user directories are needed. No unverified software was installed.'
}
