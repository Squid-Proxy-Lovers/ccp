#requires -Version 5.1
# SPDX-License-Identifier: AGPL-3.0-or-later
# Isolated installer tests: release I/O and tools are mocked; host edits use fixtures.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path -Parent $PSScriptRoot
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $root 'install.ps1'), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Installer does not parse.' }
# Load definitions only. Production has no switch that bypasses verification.
$functions = $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)
foreach ($definition in $functions) { . ([scriptblock]::Create($definition.Extent.Text)) }
# Exercise native exit/stderr handling on the current PowerShell engine before
# replacing release tools with mocks. No release code is executed by these tests.
function Test-CcpNativeExit {
    param([string]$Engine, [bool]$NativeErrorPreference)
    $ErrorActionPreference = 'Stop'
    $PSNativeCommandUseErrorActionPreference = $NativeErrorPreference
    Invoke-CcpNative $Engine @('-NoProfile', '-NonInteractive', '-Command', "[Console]::Error.WriteLine('fixture stderr'); exit 0")
    if ($LASTEXITCODE -ne 0) { throw 'Successful native command returned a failure exit code.' }
    $encoding = [Console]::OutputEncoding.CodePage
    $captured = Invoke-CcpNative $Engine @('-NoProfile', '-NonInteractive', '-Command', "[Console]::OutputEncoding = [Text.UTF8Encoding]::new(`$false); [Console]::WriteLine([char]0x00e9)") -Capture
    if ($LASTEXITCODE -ne 0 -or $captured -ne [string][char]0x00e9) { throw 'Native UTF-8 output was not captured correctly.' }
    if ([Console]::OutputEncoding.CodePage -ne $encoding) { throw 'Native capture changed caller output encoding.' }
    $nativeFailed = $false
    try { Invoke-CcpNative $Engine @('-NoProfile', '-NonInteractive', '-Command', 'exit 5') }
    catch {
        $nativeFailed = $true
        if ($_.Exception.Message -ne 'A required external command failed.') {
            throw 'Native failure did not throw the installer error.'
        }
    }
    if (-not $nativeFailed) { throw 'Native failure exit code was ignored.' }
    if ($LASTEXITCODE -ne 5) { throw 'Expected native exit code 5 was not observed.' }
    if ($PSNativeCommandUseErrorActionPreference -ne $NativeErrorPreference -or $ErrorActionPreference -ne 'Stop') {
        throw 'Native command handling changed caller error preferences.'
    }
    $missingFailed = $false
    try { Invoke-CcpNative 'ccp-fixture-missing-command-4ccac7' @() }
    catch { $missingFailed = $true }
    if (-not $missingFailed) { throw 'Missing native verifier was accepted.' }
}
$nativeRunner = ${function:Invoke-CcpNative}
$script:CcpUpgrade = $null
$systemPython = Resolve-CcpApplication 'python'
$engine = (Get-Process -Id $PID).Path
foreach ($nativeErrorPreference in @($false, $true)) { Test-CcpNativeExit $engine $nativeErrorPreference }
$CcpRepository = 'Squid-Proxy-Lovers/ccp'
$CcpClientAsset = 'ccp-client-windows-x86_64.exe'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) "ccp-installer-test-$([Guid]::NewGuid().ToString('N'))"
$profileDirectory = Join-Path $testRoot ('profile with spaces ' + [char]0x00e9)
$InstallDir = Join-Path $profileDirectory 'bin'
$script:NativeCalls = [Collections.Generic.List[object]]::new()
$script:RejectAttestation = $false
$script:RejectCodex = $false
$script:RejectPythonVersion = $false
$script:BuildFailure = ''
$script:BuildFailureReached = $false
$script:CodexFailureReached = $false
$script:BridgeBytes = 'fixture'
$script:DownloadUrls = [Collections.Generic.List[string]]::new()
$savedEnvironment = @{}
foreach ($name in @('OS', 'PROCESSOR_ARCHITECTURE', 'PROCESSOR_ARCHITEW6432', 'USERPROFILE', 'LOCALAPPDATA', 'APPDATA', 'Path')) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

function Confirm-TestCondition {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}
function Test-CcpBootstrapRef {
    param([string]$Ref, [string]$SourceVersion)
    return ($Ref -cmatch '^[0-9a-f]{40}$' -or
        ($Ref -cmatch '^v[0-9]+\.[0-9]+\.[0-9]+$' -and $Ref -ceq "v$SourceVersion"))
}
function Test-CcpApplicationResolution {
    param([string]$Root, [string]$Interpreter)
    $directory = Join-Path $Root 'application-resolution'
    $first = Join-Path $directory 'first interpreter'
    $second = Join-Path $directory 'second interpreter'
    $aliases = Join-Path $directory 'Microsoft\WindowsApps'
    $name = if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) { 'python.exe' } else { 'python' }
    $otherName = if ($name.EndsWith('.exe')) { 'ccp-fixture-app.exe' } else { 'ccp-fixture-app' }
    New-Item -ItemType Directory -Force -Path $first, $second, $aliases | Out-Null
    $firstPython = Join-Path $first $name
    $secondPython = Join-Path $second $name
    $stub = Join-Path $aliases $name
    [IO.File]::Copy($Interpreter, $firstPython, $false)
    [IO.File]::Copy($Interpreter, $secondPython, $false)
    [IO.File]::Copy($Interpreter, (Join-Path $first $otherName), $false)
    [IO.File]::Copy($Interpreter, (Join-Path $second $otherName), $false)
    # Model the zero-byte Store entry without launching it or requiring symlink privileges.
    [IO.File]::WriteAllText($stub, '')
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
        foreach ($file in @($firstPython, $secondPython, $stub, (Join-Path $first $otherName), (Join-Path $second $otherName))) {
            [IO.File]::SetUnixFileMode($file, [IO.UnixFileMode]493)
        }
    }
    $savedPath = $env:PATH
    $savedLocal = $env:LOCALAPPDATA
    try {
        $env:LOCALAPPDATA = $directory
        $env:PATH = @($first, $second, $aliases) -join [IO.Path]::PathSeparator
        Confirm-TestCondition (@(Get-Command python -CommandType Application -All).Count -eq 3) 'PATH fixture did not expose several Python applications.'
        Confirm-TestCondition ((Resolve-CcpApplication python) -eq $firstPython) 'First real Python on PATH was not selected.'
        Confirm-TestCondition (@(Resolve-CcpApplication python).Count -eq 1) 'Python resolution returned several executables.'
        Confirm-TestCondition ((Resolve-CcpApplication ccp-fixture-app) -eq (Join-Path $first $otherName)) 'Other applications did not resolve to the first PATH match.'
        $env:PATH = $aliases
        $message = ''
        try { Resolve-CcpApplication python } catch { $message = $_.Exception.Message }
        Confirm-TestCondition ($message.Contains('Install Python 3.10 or newer')) 'Only a Store alias did not fail with Python installation guidance.'
        $env:PATH = @($aliases, $first, $second) -join [IO.Path]::PathSeparator
        Confirm-TestCondition ((Resolve-CcpApplication python) -eq $firstPython) 'Store alias before a real interpreter was selected.'
        $message = ''
        try { Resolve-CcpApplication ccp-fixture-missing-app } catch { $message = $_.Exception.Message }
        Confirm-TestCondition ($message.Contains('was not found on PATH')) 'Missing application did not fail with clear guidance.'
    } finally {
        $env:PATH = $savedPath
        [Environment]::SetEnvironmentVariable('LOCALAPPDATA', $savedLocal, 'Process')
        Remove-Item -LiteralPath $directory -Recurse -Force
    }
}
function Confirm-TestFailure {
    param([scriptblock]$Action, [string]$Message)
    $failed = $false
    try { & $Action } catch { $failed = $true; $script:LastFailureMessage = $_.Exception.Message }
    Confirm-TestCondition $failed $Message
}
function Get-TestUpgradeSnapshot {
    param([string]$BinDirectory)
    $files = @{}
    $targets = Get-CcpUpgradeTarget $BinDirectory
    foreach ($key in $targets.Keys) { $files[$key] = Get-CcpFileDigest $targets[$key] }
    $state = Get-CcpInstallState $BinDirectory
    $venv = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $state.mcp_command))
    $tree = @{}
    foreach ($file in Get-ChildItem -LiteralPath $venv -File -Recurse -Force) {
        $tree[$file.FullName] = Get-CcpFileDigest $file.FullName
    }
    $parent = Join-Path $env:USERPROFILE '.ccp-mcp/generations'
    return @{
        files = $files; command = $state.mcp_command; bridge = (Get-CcpFileDigest $state.mcp_command); tree = $tree
        generations = @(Get-ChildItem -LiteralPath $parent -Directory -Force).Count
    }
}
function Confirm-TestUpgradeSnapshot {
    param($Before, [string]$BinDirectory)
    $after = Get-TestUpgradeSnapshot $BinDirectory
    foreach ($key in $Before.files.Keys) {
        Confirm-TestCondition ($after.files[$key] -eq $Before.files[$key]) "Failed upgrade changed previous $key bytes."
    }
    Confirm-TestCondition ($after.tree.Count -eq $Before.tree.Count) 'Failed upgrade changed the previous venv inventory.'
    foreach ($file in $Before.tree.Keys) {
        Confirm-TestCondition ($after.tree[$file] -eq $Before.tree[$file]) 'Failed upgrade changed previous venv bytes.'
    }
    Confirm-TestCondition ($after.command -eq $Before.command -and $after.bridge -eq $Before.bridge) 'Failed upgrade changed the active MCP bridge.'
    Confirm-TestCondition ($after.generations -eq $Before.generations) 'Failed upgrade retained a candidate generation.'
    Confirm-TestCondition (-not (Test-Path -LiteralPath (Join-Path $BinDirectory '.ccp-upgrade.json'))) 'Successful rollback retained its journal.'
    Confirm-TestCondition ((Get-Content -Encoding UTF8 -LiteralPath (Join-Path $env:USERPROFILE '.ccp-mcp/venv/Scripts/ccp-mcp-server.exe') -Raw) -eq 'legacy bridge') 'Upgrade modified the legacy venv.'
}
function Set-TestReleaseClient {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$Stage, $Manifest, [string]$ManifestPath, [string]$Body)
    if (-not $PSCmdlet.ShouldProcess($Stage, 'Prepare fixture release bytes')) { return }
    $source = Join-Path $Stage $CcpClientAsset
    [IO.File]::WriteAllText($source, $Body)
    $Manifest.artifacts[$CcpClientAsset] = @{size = (Get-Item $source).Length; sha256 = (Get-CcpFileHash -LiteralPath $source)}
    $Manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $ManifestPath
}
function Get-CcpFileHash {
    param([string]$LiteralPath)
    return (Get-FileHash -LiteralPath $LiteralPath -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Confirm-TestUpgradePair {
    param([string]$Stage, [string]$BinDirectory, [string]$Bridge)
    $hash = Get-CcpFileDigest (Join-Path $Stage $CcpClientAsset)
    Confirm-TestCondition ((Get-CcpFileDigest (Join-Path $BinDirectory 'ccp-client.exe')) -eq $hash) 'Retry did not install the expected client.'
    Confirm-TestCondition ((Get-CcpFileDigest (Join-Path $BinDirectory 'ccp.exe')) -eq $hash) 'Retry left the alias and client inconsistent.'
    $state = Get-CcpInstallState $BinDirectory
    Confirm-TestCondition ((Get-CcpFileDigest $state.mcp_client) -eq $hash) 'MCP did not retain its verified matching client.'
    Confirm-TestCondition ($state.client_sha256 -eq $hash -and $state.alias_sha256 -eq $hash) 'Ownership hashes did not converge.'
    Confirm-TestCondition ((Get-Content -Encoding UTF8 -LiteralPath $state.mcp_command -Raw) -eq $Bridge) 'Retry did not activate the matching MCP bridge.'
    $claude = Get-Content -Encoding UTF8 -LiteralPath (Join-Path $env:USERPROFILE '.claude.json') -Raw | ConvertFrom-Json
    Confirm-TestCondition ($claude.mcpServers.ccp.env.CCP_CLIENT_BIN -eq $state.mcp_client) 'Owned host config did not pin the matching client.'
    Confirm-TestCondition ($claude.mcpServers.ccp.command -eq $state.mcp_command) 'Owned host config did not switch to the matching bridge.'
    Confirm-TestCondition ($claude.other -eq 'keep') 'Upgrade lost unrelated host configuration.'
    $cursor = Get-Content -Encoding UTF8 -LiteralPath (Join-Path $env:USERPROFILE '.cursor/mcp.json') -Raw | ConvertFrom-Json
    Confirm-TestCondition ($cursor.mcpServers.ccp.command -eq $state.mcp_command) 'Owned Cursor config did not switch to the matching bridge.'
    $codex = Get-Content -Encoding UTF8 -LiteralPath (Join-Path $env:USERPROFILE '.codex/config.toml') -Raw
    $command = [regex]::Match($codex, '(?m)^command = (".*")$').Groups[1].Value | ConvertFrom-Json
    Confirm-TestCondition ($command -eq $state.mcp_command) 'Owned Codex config did not switch to the matching bridge.'
    Confirm-TestCondition (-not (Test-Path -LiteralPath (Join-Path $BinDirectory '.ccp-upgrade.json'))) 'Retry did not finish recovery.'
}
function Test-CcpUpgradeRollback {
    param([string]$Stage, [string]$BinDirectory, $Manifest, [string]$ManifestPath)
    $targets = Get-CcpUpgradeTarget $BinDirectory
    # Register a byte-identical verified alias even when the host already has ccp on PATH.
    [IO.File]::Copy($targets.client, $targets.alias, $true)
    $record = Get-CcpInstallState $BinDirectory
    $record.alias_sha256 = Get-CcpFileDigest $targets.alias
    [IO.File]::WriteAllText($targets.install, ($record | ConvertTo-Json))
    $script:SavedUpgradePublisher = ${function:Publish-CcpFile}
    $script:PublishFailureDestination = ''
    $script:PublishFailureFired = $false
    $script:BlockRestore = $false
    $script:UpgradeClientPath = $targets.client
    Set-Item Function:Publish-CcpFile {
        param([string]$Source, [string]$Destination)
        if ($script:BlockRestore -and $Destination -eq $script:UpgradeClientPath -and (Split-Path -Leaf $Source) -eq 'client') {
            throw 'Fixture blocked rollback until retry.'
        }
        if ($Destination -eq $script:PublishFailureDestination -and -not $script:PublishFailureFired) {
            $script:PublishFailureFired = $true
            throw 'Fixture interrupted publication.'
        }
        & $script:SavedUpgradePublisher $Source $Destination
    }
    try {
        $before = Get-TestUpgradeSnapshot $BinDirectory
        Set-TestReleaseClient $Stage $Manifest $ManifestPath 'transactional client'
        $script:BridgeBytes = 'transactional bridge'
        $script:PublishFailureDestination = $targets.alias
        Confirm-TestFailure { Install-CcpWindows stable '' $BinDirectory } 'Second executable publication did not fail.'
        Confirm-TestCondition $script:PublishFailureFired 'Second executable failure injection was not reached.'
        Confirm-TestUpgradeSnapshot $before $BinDirectory
        $script:PublishFailureDestination = ''
        Install-CcpWindows stable '' $BinDirectory
        Confirm-TestUpgradePair $Stage $BinDirectory $script:BridgeBytes
        Confirm-TestCondition ((Get-CcpFileDigest $before.command) -eq $before.bridge) 'Successful upgrade modified the previous venv.'
        # The recorded alias is still ours if a prior attempt changed only the client.
        [IO.File]::WriteAllText($targets.client, 'partial previous client')
        Install-CcpWindows stable '' $BinDirectory
        Confirm-TestUpgradePair $Stage $BinDirectory $script:BridgeBytes
        $before = Get-TestUpgradeSnapshot $BinDirectory
        Set-TestReleaseClient $Stage $Manifest $ManifestPath 'next transactional client'
        $script:BridgeBytes = 'next transactional bridge'
        foreach ($failure in @('venv', 'pip', 'import', 'entrypoint', 'client', 'alias', 'claude', 'codex', 'install')) {
            $script:BuildFailure = if ($failure -in @('venv', 'pip', 'import', 'entrypoint')) { $failure } else { '' }
            $script:RejectCodex = $failure -eq 'codex'
            $script:PublishFailureDestination = if ($failure -in @('client', 'alias', 'claude', 'install')) { $targets[$failure] } else { '' }
            $script:PublishFailureFired = $false
            $script:BuildFailureReached = $false; $script:CodexFailureReached = $false
            try {
                Confirm-TestFailure { Install-CcpWindows stable '' $BinDirectory } "Upgrade unexpectedly passed $failure failure."
                if ($script:BuildFailure) { Confirm-TestCondition $script:BuildFailureReached "Build failure at $failure was not reached." }
                if ($script:RejectCodex) { Confirm-TestCondition $script:CodexFailureReached 'Codex failure was not reached.' }
                if ($script:PublishFailureDestination) { Confirm-TestCondition $script:PublishFailureFired "Publication failure at $failure was not reached." }
                Confirm-TestUpgradeSnapshot $before $BinDirectory
            } finally {
                $script:BuildFailure = ''; $script:RejectCodex = $false; $script:PublishFailureDestination = ''
            }
        }
        Install-CcpWindows stable '' $BinDirectory
        Confirm-TestUpgradePair $Stage $BinDirectory $script:BridgeBytes
        $before = Get-TestUpgradeSnapshot $BinDirectory
        Set-TestReleaseClient $Stage $Manifest $ManifestPath 'recovered client'
        $script:BridgeBytes = 'recovered bridge'
        $script:PublishFailureDestination = $targets.alias; $script:PublishFailureFired = $false; $script:BlockRestore = $true
        Confirm-TestFailure { Install-CcpWindows stable '' $BinDirectory } 'Blocked rollback did not fail closed.'
        Confirm-TestCondition (Test-Path -LiteralPath (Join-Path $BinDirectory '.ccp-upgrade.json')) 'Blocked rollback discarded recovery data.'
        Confirm-TestCondition ($script:LastFailureMessage.Contains('mid-recovery') -and $script:LastFailureMessage.Contains('re-run the same command')) 'Blocked rollback did not explain recovery.'
        $callsBefore = $script:NativeCalls.Count; $downloadsBefore = $script:DownloadUrls.Count
        Confirm-TestFailure { Install-CcpWindows stable '' $BinDirectory } 'Still-blocked recovery did not fail closed.'
        Confirm-TestCondition ($script:NativeCalls.Count -eq $callsBefore -and $script:DownloadUrls.Count -eq $downloadsBefore) 'A retry started tool or network work before recovery completed.'
        Confirm-TestCondition ((Get-CcpFileDigest $before.command) -eq $before.bridge) 'Blocked rollback changed the previous MCP bridge.'
        $script:BlockRestore = $false; $script:PublishFailureDestination = ''
        Install-CcpWindows stable '' $BinDirectory
        Confirm-TestUpgradePair $Stage $BinDirectory $script:BridgeBytes
        Confirm-TestCondition ((Get-TestUpgradeSnapshot $BinDirectory).generations -eq [Math]::Min(2, ($before.generations + 1))) 'Recovery did not remove the failed candidate.'
    } finally {
        $script:BlockRestore = $false; $script:PublishFailureDestination = ''; $script:BuildFailure = ''; $script:RejectCodex = $false
        Set-Item Function:Publish-CcpFile $script:SavedUpgradePublisher
    }
}
function Get-TestPendingUpgrade {
    param([string]$BinDirectory)
    $pointer = Get-Content -Encoding UTF8 -LiteralPath (Join-Path $BinDirectory '.ccp-upgrade.json') -Raw | ConvertFrom-Json
    return (Get-Content -Encoding UTF8 -LiteralPath (Join-Path $pointer.directory 'state.json') -Raw | ConvertFrom-Json)
}
function Test-CcpRestartableRecovery {
    param([string]$Stage, [string]$BinDirectory, $Manifest, [string]$ManifestPath)
    $targets = Get-CcpUpgradeTarget $BinDirectory
    $script:R3Publisher = ${function:Publish-CcpFile}; $script:R3Saver = ${function:Save-CcpUpgradeState}
    $script:R3Remover = ${function:Remove-CcpPrivateStage}; $script:R3Tree = ${function:Remove-CcpTree}
    $script:R3Pointer = ${function:Save-CcpCleanupPointer}; $script:R3Journal = ${function:Remove-CcpUpgradeJournal}
    $script:R3Client = $targets.client; $script:R3Alias = $targets.alias
    $script:R3Block = $false; $script:R3AliasFault = $false; $script:R3AliasFired = $false
    $script:R3Phase = ''; $script:R3Fired = $false; $script:R3Candidate = ''; $script:R3Directory = ''
    $script:R3AliasWrites = 0
    Set-Item Function:Publish-CcpFile {
        param([string]$Source, [string]$Destination)
        if ($Destination -eq $script:R3Alias) { $script:R3AliasWrites++ }
        if ($script:R3Block -and $Destination -eq $script:R3Client -and (Split-Path -Leaf $Source) -eq 'client') { throw 'Blocked restore.' }
        if ($script:R3AliasFault -and $Destination -eq $script:R3Alias -and -not $script:R3AliasFired) {
            $script:R3AliasFired = $true; throw 'Second publish failed.'
        }
        & $script:R3Publisher $Source $Destination
    }
    Set-Item Function:Save-CcpUpgradeState {
        param($State)
        & $script:R3Saver $State
        if ($script:R3Phase -eq 'checkpoint' -and $State.cleanup_ready -and -not $State.committed -and -not $script:R3Fired) {
            $script:R3Fired = $true; throw 'Stopped after durable cleanup checkpoint.'
        }
    }
    Set-Item Function:Save-CcpCleanupPointer {
        param([string]$BinDirectory, [string]$Directory)
        & $script:R3Pointer $BinDirectory $Directory
        if ($script:R3Phase -eq 'pointer' -and -not $script:R3Fired) { $script:R3Fired = $true; throw 'Stopped after cleanup pointer.' }
    }
    Set-Item Function:Remove-CcpTree {
        param([string]$Path)
        if ($script:R3Phase -eq 'partial-stage' -and $Path -eq $script:R3Directory -and -not $script:R3Fired) {
            [IO.File]::Delete((Join-Path $Path 'state.json'))
            $script:R3Fired = $true; throw 'Stopped partway through snapshot deletion.'
        }
        & $script:R3Tree $Path
    }
    Set-Item Function:Remove-CcpPrivateStage {
        param([string]$Path)
        & $script:R3Remover $Path
        if (-not $script:R3Fired -and (($script:R3Phase -eq 'candidate' -and $Path -eq $script:R3Candidate) -or
            ($script:R3Phase -eq 'stage' -and $Path -eq $script:R3Directory))) {
            $script:R3Fired = $true; throw 'Stopped after directory deletion.'
        }
    }
    Set-Item Function:Remove-CcpUpgradeJournal {
        param([string]$BinDirectory)
        & $script:R3Journal $BinDirectory
        if ($script:R3Phase -eq 'journal' -and -not $script:R3Fired) { $script:R3Fired = $true; throw 'Stopped after journal deletion.' }
    }
    try {
        foreach ($phase in @('checkpoint', 'candidate', 'pointer', 'partial-stage', 'stage', 'journal', 'foreign-alias', 'foreign-client')) {
            Set-TestReleaseClient $Stage $Manifest $ManifestPath "cleanup client $phase"
            $script:BridgeBytes = "cleanup bridge $phase"
            $script:R3Block = $true; $script:R3AliasFault = $true; $script:R3AliasFired = $false
            Confirm-TestFailure { Install-CcpWindows stable '' $BinDirectory } 'Recovery setup did not block.'
            Confirm-TestCondition ($script:LastFailureMessage.Contains('mid-recovery')) 'Blocked recovery message was not clear.'
            $pending = Get-TestPendingUpgrade $BinDirectory
            $script:R3Candidate = $pending.candidate; $script:R3Directory = $pending.directory
            Invoke-CcpRetention $BinDirectory
            Confirm-TestCondition (Test-Path -LiteralPath $pending.candidate) 'Retention pruned a rollback generation.'
            $script:R3Block = $false; $script:R3Phase = $phase; $script:R3Fired = $false
            if ($phase -eq 'foreign-alias') {
                [IO.File]::WriteAllText($targets.alias, 'unrelated replacement executable')
                $writes = $script:R3AliasWrites
                $warnings = @(Invoke-CcpRecovery $BinDirectory 3>&1)
                Confirm-TestCondition ((Get-Content -Encoding UTF8 -LiteralPath $targets.alias -Raw) -eq 'unrelated replacement executable') 'Recovery overwrote an unrelated alias.'
                Confirm-TestCondition ($script:R3AliasWrites -eq $writes) 'Recovery attempted to publish over an unrelated alias.'
                Confirm-TestCondition ($null -eq (Get-CcpInstallState $BinDirectory).alias_sha256) 'Recovery retained alias ownership.'
                Confirm-TestCondition (($warnings -join ' ').Contains('unrelated replacement')) 'Recovery did not warn about the replacement alias.'
                Install-CcpWindows stable '' $BinDirectory
                Confirm-TestCondition ($script:R3AliasWrites -eq $writes) 'Retry adopted the unrelated alias.'
                # Even unrecorded bytes identical to the release are not ownership proof.
                [IO.File]::Copy($targets.client, $targets.alias, $true)
                Install-CcpWindows stable '' $BinDirectory
                Confirm-TestCondition ($script:R3AliasWrites -eq $writes -and $null -eq (Get-CcpInstallState $BinDirectory).alias_sha256) 'An unrecorded pre-existing alias was adopted.'
                $record = Get-CcpInstallState $BinDirectory; $record.alias_sha256 = Get-CcpFileDigest $targets.alias
                [IO.File]::WriteAllText($targets.install, ($record | ConvertTo-Json))
            } elseif ($phase -eq 'foreign-client') {
                [IO.File]::WriteAllText($targets.client, 'unrelated client replacement')
                $warnings = @(Confirm-TestFailure { Invoke-CcpRecovery $BinDirectory } 'Unknown client replacement was overwritten.' 3>&1)
                Confirm-TestCondition ((Get-Content -Encoding UTF8 -LiteralPath $targets.client -Raw) -eq 'unrelated client replacement') 'Recovery overwrote unknown destination bytes.'
                Confirm-TestCondition (($warnings -join ' ').Contains('left untouched')) 'Recovery did not warn about unknown destination bytes.'
                # User resolves the external change to known proposed bytes before retry.
                [IO.File]::Copy((Join-Path $Stage $CcpClientAsset), $targets.client, $true)
                Install-CcpWindows stable '' $BinDirectory
                Confirm-TestUpgradePair $Stage $BinDirectory $script:BridgeBytes
            } else {
                Confirm-TestFailure { Invoke-CcpRecovery $BinDirectory } "Cleanup interruption at $phase did not fail."
                Confirm-TestCondition $script:R3Fired "Cleanup interruption at $phase was not reached."
                $script:R3Phase = ''
                Install-CcpWindows stable '' $BinDirectory
                Confirm-TestUpgradePair $Stage $BinDirectory $script:BridgeBytes
                Confirm-TestCondition (-not (Test-Path -LiteralPath $pending.directory)) 'Recovery retained the transaction directory.'
                Confirm-TestCondition (-not (Test-Path -LiteralPath $pending.candidate)) 'Recovery retained the rollback candidate.'
            }
            $script:R3Phase = ''; $script:R3AliasFault = $false
        }
    } finally {
        Set-Item Function:Publish-CcpFile $script:R3Publisher; Set-Item Function:Save-CcpUpgradeState $script:R3Saver
        Set-Item Function:Remove-CcpPrivateStage $script:R3Remover; Set-Item Function:Remove-CcpTree $script:R3Tree
        Set-Item Function:Save-CcpCleanupPointer $script:R3Pointer; Set-Item Function:Remove-CcpUpgradeJournal $script:R3Journal
    }
}
function Test-CcpBoundedRetention {
    param([string]$Stage, [string]$BinDirectory, $Manifest, [string]$ManifestPath)
    $parent = Join-Path $env:USERPROFILE '.ccp-mcp/generations'
    $unknown = New-CcpPrivateStage $parent
    [IO.File]::WriteAllText((Join-Path $unknown 'user.txt'), 'unrecorded user directory')
    $foreignBackup = Join-Path $env:USERPROFILE ('.claude.json.backup.20000101T000000000Z.' + [Guid]::NewGuid().ToString('N'))
    [IO.File]::WriteAllText($foreignBackup, 'unrecorded user backup')
    $outside = Join-Path (Split-Path -Parent $env:USERPROFILE) 'retention-outside'
    New-Item -ItemType Directory -Force -Path $outside | Out-Null
    [IO.File]::WriteAllText((Join-Path $outside 'keep.txt'), 'do not traverse')
    $record = Get-CcpRetention $BinDirectory
    $old = $record.generations | Sort-Object id | Select-Object -First 1
    $type = if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) { 'Junction' } else { 'SymbolicLink' }
    New-Item -ItemType $type -Path (Join-Path $old.directory 'external-link') -Target $outside | Out-Null
    $script:R3RetentionTree = ${function:Remove-CcpTree}
    $script:R3RetentionOld = $old.directory
    $script:R3RetentionBlocked = $true
    $script:R3RetentionReached = $false
    Set-Item Function:Remove-CcpTree {
        param([string]$Path)
        if ($Path -eq $script:R3RetentionOld -and $script:R3RetentionBlocked) {
            [IO.File]::Delete((Join-Path $Path '.ccp-generation.json'))
            $script:R3RetentionReached = $true
            throw 'Interrupted recorded generation deletion.'
        }
        & $script:R3RetentionTree $Path
    }
    try {
        Set-TestReleaseClient $Stage $Manifest $ManifestPath 'interrupted pruning client'
        $script:BridgeBytes = 'interrupted pruning bridge'
        Install-CcpWindows stable '' $BinDirectory
        Confirm-TestCondition $script:R3RetentionReached 'Generation deletion interruption was not reached.'
        $installed = Get-CcpInstallState $BinDirectory
        Confirm-TestCondition (Test-Path -LiteralPath $installed.mcp_command) 'Retention interruption changed the active generation.'
        $calls = $script:NativeCalls.Count; $downloads = $script:DownloadUrls.Count
        Confirm-TestFailure { Install-CcpWindows stable '' $BinDirectory } 'Blocked retention allowed another upgrade.'
        Confirm-TestCondition ($script:NativeCalls.Count -eq $calls -and $script:DownloadUrls.Count -eq $downloads) 'Pending cleanup did not finish before tool/network work.'
        $script:R3RetentionBlocked = $false
        Install-CcpWindows stable '' $BinDirectory
        Confirm-TestCondition (-not (Test-Path -LiteralPath $old.directory)) 'Partially deleted generation could not be pruned on retry.'
    } finally { Set-Item Function:Remove-CcpTree $script:R3RetentionTree }
    $script:R3BackupRegistrar = ${function:Register-CcpBackup}
    $script:R3Backups = [Collections.Generic.List[object]]::new()
    Set-Item Function:Register-CcpBackup {
        param([string]$Path, [string]$Backup, [string]$Hash)
        $script:R3Backups.Add(@{config = $Path; path = $Backup})
        & $script:R3BackupRegistrar $Path $Backup $Hash
    }
    try {
        for ($iteration = 1; $iteration -le 6; $iteration++) {
            $before = Get-CcpInstallState $BinDirectory
            $previous = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $before.mcp_command))
            Set-TestReleaseClient $Stage $Manifest $ManifestPath "retained client $iteration"
            $script:BridgeBytes = "retained bridge $iteration"
            Install-CcpWindows stable '' $BinDirectory
            $record = Get-CcpRetention $BinDirectory
            $active = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent (Get-CcpInstallState $BinDirectory).mcp_command))
            Confirm-TestCondition (@($record.generations).Count -eq 2) 'Generation retention did not keep exactly two.'
            Confirm-TestCondition ($record.generations.directory -contains $active -and $record.generations.directory -contains $previous) 'Retention removed active or previous generation.'
            Confirm-TestCondition (@(Get-ChildItem -LiteralPath $parent -Directory -Force).Count -eq 3) 'Recorded generation pruning did not remove old directories.'
            foreach ($key in @('claude', 'cursor', 'codex')) {
                Confirm-TestCondition (@($record.backups | Where-Object config -EQ $key).Count -le 3) 'Configuration backups exceeded their bound.'
            }
            Confirm-TestCondition ((Get-Content -Encoding UTF8 -LiteralPath (Join-Path $unknown 'user.txt') -Raw) -eq 'unrecorded user directory') 'Retention pruned an unrecorded directory.'
            Confirm-TestCondition ((Get-Content -Encoding UTF8 -LiteralPath $foreignBackup -Raw) -eq 'unrecorded user backup') 'Retention pruned an unrecorded backup.'
            Confirm-TestCondition ((Get-Content -Encoding UTF8 -LiteralPath (Join-Path $outside 'keep.txt') -Raw) -eq 'do not traverse') 'Generation pruning followed a nested junction.'
        }
        $script:R3RetentionSaver = ${function:Save-CcpRetention}
        $script:R3BackupDeleteBlocked = $true; $script:R3BackupDeleteReached = $false
        Set-Item Function:Save-CcpRetention {
            param($Record, [string]$BinDirectory)
            $prior = Get-CcpRetention $BinDirectory
            $missing = @($prior.backups | Where-Object { -not (Test-Path -LiteralPath $_.path) })
            if ($script:R3BackupDeleteBlocked -and $missing.Count -and
                @($Record.backups | Where-Object config -EQ 'claude').Count -le 3) {
                $script:R3BackupDeleteReached = $true
                throw 'Interrupted inventory update after backup deletion.'
            }
            & $script:R3RetentionSaver $Record $BinDirectory
        }
        try {
            Set-TestReleaseClient $Stage $Manifest $ManifestPath 'interrupted backup client'
            $script:BridgeBytes = 'interrupted backup bridge'
            Confirm-TestFailure { Install-CcpWindows stable '' $BinDirectory } 'Backup deletion interruption did not fail.'
            Confirm-TestCondition $script:R3BackupDeleteReached 'Backup inventory interruption was not reached.'
            $script:R3BackupDeleteBlocked = $false
            Install-CcpWindows stable '' $BinDirectory
            $record = Get-CcpRetention $BinDirectory
            Confirm-TestCondition (@($record.generations).Count -eq 2) 'Backup recovery changed generation retention.'
        } finally { Set-Item Function:Save-CcpRetention $script:R3RetentionSaver }
        $targets = Get-CcpUpgradeTarget $BinDirectory
        foreach ($key in @('claude', 'codex')) {
            $latest = @($script:R3Backups | Where-Object config -EQ $targets[$key] | Select-Object -Last 3)
            $kept = @($record.backups | Where-Object config -EQ $key)
            Confirm-TestCondition ($kept.Count -eq 3) 'Retention kept fewer than the three recent backups.'
            foreach ($entry in $kept) { Confirm-TestCondition ($latest.path -contains $entry.path) 'Retention did not keep the newest recorded backups.' }
        }
    } finally { Set-Item Function:Register-CcpBackup $script:R3BackupRegistrar }
}
function Invoke-CcpNative {
    param([string]$Executable, [string[]]$ArgumentList, [switch]$Capture)
    $script:NativeCalls.Add(@{ Executable = $Executable; Arguments = $ArgumentList })
    if ($script:RejectPythonVersion -and $ArgumentList -contains '-c') { throw 'Fixture unsupported Python version.' }
    if ($Executable -eq 'gh' -and $script:RejectAttestation) { throw 'Fixture attestation failure.' }
    if ($Executable -eq 'codex') {
        if ($script:RejectCodex) { $script:CodexFailureReached = $true; throw 'Fixture interrupted Codex update.' }
        $path = Join-Path $env:CODEX_HOME 'config.toml'
        $text = Get-Content -Encoding UTF8 -LiteralPath $path -Raw
        if ($ArgumentList -contains 'get') {
            if (-not $Capture) { throw 'Host query output must be captured.' }
            $command = [regex]::Match($text, '(?m)^command = (".*")$').Groups[1].Value | ConvertFrom-Json
            $client = [regex]::Match($text, '(?m)^CCP_CLIENT_BIN = (".*")$').Groups[1].Value | ConvertFrom-Json
            return (@{transport = @{type = 'stdio'; command = $command; args = @(); env = @{CCP_CLIENT_BIN = $client}}} | ConvertTo-Json -Depth 5)
        }
        $text = [regex]::Replace($text, '(?ms)^\[mcp_servers\.ccp(?:\.env)?\]\r?\n.*?(?=^\[|\z)', '')
        $command = $ArgumentList[-1] | ConvertTo-Json -Compress
        $client = ($ArgumentList | Where-Object { $_.StartsWith('CCP_CLIENT_BIN=') }).Substring(15) | ConvertTo-Json -Compress
        [IO.File]::WriteAllText($path, $text + "`n[mcp_servers.ccp]`ncommand = $command`n[mcp_servers.ccp.env]`nCCP_CLIENT_BIN = $client`n")
    }
    if ($ArgumentList -contains 'venv') {
        $scripts = Join-Path $ArgumentList[-1] 'Scripts'
        New-Item -ItemType Directory -Force -Path $scripts | Out-Null
        [IO.File]::WriteAllText((Join-Path $scripts 'python.exe'), 'fixture')
        if ($script:BuildFailure -eq 'venv') { $script:BuildFailureReached = $true; throw 'Fixture interrupted venv creation.' }
    }
    if ($ArgumentList -contains 'pip') {
        $command = Join-Path (Split-Path $Executable) 'ccp-mcp-server.exe'
        [IO.File]::WriteAllText($command, $script:BridgeBytes)
        if ($script:BuildFailure -eq 'pip') { $script:BuildFailureReached = $true; [IO.File]::Delete($command); throw 'Fixture interrupted package install.' }
        if ($script:BuildFailure -eq 'entrypoint') { $script:BuildFailureReached = $true; [IO.File]::Delete($command) }
    }
    if ($ArgumentList -contains '-c' -and ($ArgumentList -join ' ').Contains('importlib.metadata') -and $script:BuildFailure -eq 'import') {
        $script:BuildFailureReached = $true
        throw 'Fixture package import failed.'
    }
}
function Save-CcpReleaseFile {
    param([string]$Uri, [string]$OutFile)
    $script:DownloadUrls.Add($Uri)
    Copy-Item -LiteralPath (Join-Path $fixture ($Uri.Split('/')[-1])) -Destination $OutFile
}
function Get-CcpReleaseResponse {
    param([string]$Uri)
    if ($Uri.EndsWith('/latest')) {
        return @{ tag_name = 'v0.2.0'; draft = $false; prerelease = $false }
    }
    return @(
        @{ tag_name = 'v0.2.0'; draft = $false; prerelease = $false; published_at = '2026-01-04' },
        @{ tag_name = 'nightly-2026-01-02-aaaaaaaaaaaa'; draft = $false; prerelease = $true; published_at = '2026-01-02' },
        @{ tag_name = 'nightly-2026-01-03-aaaaaaaaaaaa'; draft = $true; prerelease = $true; published_at = '2026-01-03' }
    )
}
# The full flow exercises PATH intent without altering the user's registry.
$setUserPath = ${function:Set-CcpUserPath}
function Set-CcpUserPath {
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$BinDirectory)
    if (-not $PSCmdlet.ShouldProcess($BinDirectory, 'Configure CCP')) { return }
    $script:ObservedBinDirectory = $BinDirectory
}

try {
    New-Item -ItemType Directory -Force -Path $profileDirectory | Out-Null
    $env:USERPROFILE = $profileDirectory
    $env:LOCALAPPDATA = $profileDirectory
    $env:APPDATA = $profileDirectory
    Test-CcpApplicationResolution $testRoot $systemPython
    $script:RejectPythonVersion = $true
    try {
        $message = ''
        try { Resolve-CcpPython } catch { $message = $_.Exception.Message }
        Confirm-TestCondition ($message.Contains('Install Python 3.10 or newer')) 'Unsupported Python version did not fail with installation guidance.'
    } finally { $script:RejectPythonVersion = $false }
    $fixture = New-CcpPrivateStage $profileDirectory
    $packagedAssets = @($CcpClientAsset, 'ccp-mcp.tar.gz')
    foreach ($platform in @('linux-x86_64', 'linux-aarch64', 'darwin-x86_64', 'darwin-aarch64')) {
        $packagedAssets += @("ccp-client-$platform", "ccp-server-$platform")
    }
    foreach ($name in $packagedAssets) {
        $path = Join-Path $fixture $name
        [IO.File]::WriteAllText($path, "fixture bytes for $name")
    }
    # Use the actual release producer, then retain mutable records for fault cases.
    & $nativeRunner 'python' @((Join-Path $root 'scripts/nightly.py'), 'prepare', '--dist', $fixture, '--sha', ('a' * 40))
    Confirm-TestCondition ($LASTEXITCODE -eq 0) 'Release fixture packaging failed.'
    $manifestPath = Join-Path $fixture 'ccp-release.json'
    $packaged = Get-Content -Encoding UTF8 -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    $manifest = @{
        format = $packaged.format; repository = $packaged.repository; source_sha = $packaged.source_sha
        package_version = $packaged.package_version; platforms = $packaged.platforms
        contracts = $packaged.contracts; artifacts = @{}
    }
    foreach ($property in $packaged.artifacts.PSObject.Properties) {
        $manifest.artifacts[$property.Name] = @{ size = $property.Value.size; sha256 = $property.Value.sha256 }
    }
    Confirm-TestCondition ($manifest.artifacts.Count -eq 11) 'Fixture differs from the release producer inventory.'
    Confirm-TestCondition ($manifest.platforms.Count -eq 5) 'Fixture is missing real release platforms.'
    Confirm-TestCondition (Test-CcpBootstrapRef ('a' * 40) '1.2.3') 'Full commit SHA was rejected.'
    Confirm-TestCondition (Test-CcpBootstrapRef 'v1.2.3' '1.2.3') 'Matching release tag was rejected.'
    Confirm-TestCondition (-not (Test-CcpBootstrapRef 'v1.2.3' '1.2.4')) 'Mismatched release tag was accepted.'
    Confirm-TestCondition (-not (Test-CcpBootstrapRef 'main' '1.2.3')) 'Moving main ref was accepted.'
    foreach ($document in @('README.md', 'docs/client.md')) {
        $instructions = Get-Content -Encoding UTF8 -LiteralPath (Join-Path $root $document) -Raw
        $urls = [regex]::Matches($instructions, 'https://raw\.githubusercontent\.com/squid-proxy-lovers/ccp/([^/\s]+)/install\.ps1')
        Confirm-TestCondition ($urls.Count -gt 0) 'Bootstrap instructions are missing a pinned URL.'
        foreach ($url in $urls) {
            Confirm-TestCondition (Test-CcpBootstrapRef $url.Groups[1].Value $packaged.package_version) 'Bootstrap ref must be a full SHA or a release tag matching the source version.'
        }
        Confirm-TestCondition (-not $instructions.Contains('/ccp/main/install.ps1')) 'Bootstrap instructions still execute the moving main branch.'
    }
    [IO.File]::WriteAllText((Join-Path $fixture 'ccp-provenance.json'), 'fixture attestation')
    Confirm-TestCondition ((Get-CcpReleaseTag stable '') -eq 'v0.2.0') 'Stable selection failed.'
    Confirm-TestCondition ((Get-CcpReleaseTag nightly '') -eq 'nightly-2026-01-02-aaaaaaaaaaaa') 'Nightly selection included a draft.'
    Confirm-TestFailure { Get-CcpReleaseTag stable 'nightly-2026-01-02-aaaaaaaaaaaa' } 'Stable accepted a nightly pin.'
    Confirm-TestFailure { Get-CcpReleaseTag nightly '../bad' } 'Invalid tag accepted.'
    Confirm-CcpRelease $fixture stable 'v0.2.0'
    $arguments = $script:NativeCalls[-1].Arguments
    foreach ($flag in @('--bundle', '--hostname', '--repo', '--cert-identity', '--cert-oidc-issuer',
            '--source-ref', '--source-digest', '--signer-digest', '--deny-self-hosted-runners')) {
        Confirm-TestCondition ($arguments -contains $flag) "Missing publisher policy: $flag"
    }
    Confirm-TestCondition ($arguments -contains 'refs/tags/v0.2.0') 'Wrong stable provenance ref.'
    Confirm-CcpRelease $fixture nightly 'nightly-2026-01-02-aaaaaaaaaaaa'
    Confirm-TestCondition ($script:NativeCalls[-1].Arguments -contains 'refs/heads/main') 'Wrong nightly provenance ref.'
    Confirm-TestFailure { Confirm-CcpRelease $fixture stable 'v0.3.0' } 'Stable mismatch accepted.'
    Confirm-TestFailure { Confirm-CcpRelease $fixture nightly 'nightly-2026-01-02-bbbbbbbbbbbb' } 'Nightly mismatch accepted.'
    $manifest.repository = 'untrusted/ccp'
    $manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $manifestPath
    Confirm-TestFailure { Confirm-CcpRelease $fixture stable 'v0.2.0' } 'Foreign publisher accepted.'
    $manifest.repository = $CcpRepository
    $manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $manifestPath
    $script:RejectAttestation = $true
    Confirm-TestFailure { Confirm-CcpRelease $fixture stable 'v0.2.0' } 'Attestation failure accepted.'
    $script:RejectAttestation = $false
    foreach ($name in @($CcpClientAsset, 'ccp-mcp.tar.gz', 'build-info.json')) {
        $path = Join-Path $fixture $name
        $original = [IO.File]::ReadAllBytes($path)
        $tampered = $original.Clone()
        $tampered[0] = $tampered[0] -bxor 1
        [IO.File]::WriteAllBytes($path, $tampered)
        Confirm-TestFailure { Confirm-CcpRelease $fixture stable 'v0.2.0' } "Tampered $name accepted."
        [IO.File]::WriteAllBytes($path, $original)
    }
    $manifest.artifacts.Remove($CcpClientAsset)
    $manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $manifestPath
    Confirm-TestFailure { Confirm-CcpRelease $fixture stable 'v0.2.0' } 'Missing client manifest entry accepted.'
    $manifest.artifacts[$CcpClientAsset] = @{
        size = (Get-Item (Join-Path $fixture $CcpClientAsset)).Length
        sha256 = (Get-FileHash (Join-Path $fixture $CcpClientAsset)).Hash.ToLowerInvariant()
    }
    $manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $manifestPath

    $env:OS = 'Windows_NT'
    $env:PROCESSOR_ARCHITECTURE = 'AMD64'
    $env:USERPROFILE = $profileDirectory
    $env:LOCALAPPDATA = $profileDirectory
    $env:APPDATA = $profileDirectory
    [IO.File]::WriteAllText((Join-Path $profileDirectory '.claude.json'), '{"other":"keep"}')
    New-Item -ItemType Directory -Path (Join-Path $profileDirectory '.cursor'), (Join-Path $profileDirectory '.codex') | Out-Null
    [IO.File]::WriteAllText((Join-Path $profileDirectory '.codex/config.toml'), '# keep')
    $legacyScripts = Join-Path $profileDirectory '.ccp-mcp/venv/Scripts'
    New-Item -ItemType Directory -Force -Path $legacyScripts | Out-Null
    [IO.File]::WriteAllText((Join-Path $legacyScripts 'python.exe'), 'legacy interpreter')
    [IO.File]::WriteAllText((Join-Path $legacyScripts 'ccp-mcp-server.exe'), 'legacy bridge')
    $script:RejectAttestation = $true
    Confirm-TestFailure { Install-CcpWindows stable '' $InstallDir } 'Full flow ignored attestation failure.'
    Confirm-TestCondition (-not (Test-Path -LiteralPath $InstallDir)) 'Failed verification created the install directory.'
    $script:RejectAttestation = $false
    Install-CcpWindows stable '' $InstallDir
    Confirm-TestCondition ($script:ObservedBinDirectory -eq $InstallDir) 'PATH was not configured.'
    Confirm-TestCondition (Test-Path -LiteralPath (Join-Path $InstallDir 'ccp-client.exe')) 'Client was not installed.'
    $claude = Get-Content -Encoding UTF8 -LiteralPath (Join-Path $profileDirectory '.claude.json') -Raw | ConvertFrom-Json
    Confirm-TestCondition ($claude.other -eq 'keep') 'Unrelated host setting was lost.'
    Confirm-TestCondition ($claude.mcpServers.ccp.env.CCP_CLIENT_BIN -eq (Get-CcpInstallState $InstallDir).mcp_client) 'MCP client path was not encoded correctly.'
    Confirm-TestCondition ($script:NativeCalls.Executable -contains 'codex') 'Codex MCP command was not called.'
    $claudeBackup = @(Get-ChildItem -LiteralPath $profileDirectory -Force -Filter '.claude.json.backup.*')
    Confirm-TestCondition ($claudeBackup.Count -eq 1) 'Claude config backup was not written.'
    Confirm-TestCondition (([IO.File]::ReadAllText($claudeBackup[0].FullName)) -eq '{"other":"keep"}') 'Claude backup differs from the original bytes.'
    $codexDirectory = Join-Path $profileDirectory '.codex'
    $codexPath = Join-Path $codexDirectory 'config.toml'
    $codexBackup = @(Get-ChildItem -LiteralPath $codexDirectory -Filter 'config.toml.backup.*')
    Confirm-TestCondition ($codexBackup.Count -eq 1) 'Codex config backup was not written.'
    Confirm-TestCondition (([IO.File]::ReadAllText($codexBackup[0].FullName)) -eq '# keep') 'Codex backup differs from the original bytes.'
    Confirm-TestCondition ((Get-Content -Encoding UTF8 -LiteralPath $codexPath -Raw).Contains('# keep')) 'Codex lost unrelated configuration.'
    $beforeInterruptedConfig = [IO.File]::ReadAllText($codexPath)
    $script:RejectCodex = $true
    Confirm-TestFailure { Set-CcpCodexConfig $codexPath 'fixture' 'fixture' (Get-CcpInstallState $InstallDir).mcp_command (Get-CcpInstallState $InstallDir).mcp_client } 'Interrupted Codex update was accepted.'
    $script:RejectCodex = $false
    Confirm-TestCondition (([IO.File]::ReadAllText($codexPath)) -eq $beforeInterruptedConfig) 'Interrupted update replaced the existing Codex config.'
    Confirm-TestCondition (@(Get-ChildItem -LiteralPath $codexDirectory -Force -Filter '.ccp-stage-*').Count -eq 0) 'Interrupted update leaked staging files.'
    $pipCalls = @($script:NativeCalls | Where-Object { $_.Arguments -contains 'pip' })
    Confirm-TestCondition ($pipCalls[0].Executable.EndsWith((Join-Path 'Scripts' 'python.exe'))) 'MCP did not use the venv Python.'
    $downloadedClient = Join-Path $fixture $CcpClientAsset
    [IO.File]::WriteAllText($downloadedClient, 'updated verified client')
    $manifest.artifacts[$CcpClientAsset] = @{
        size = (Get-Item $downloadedClient).Length
        sha256 = (Get-FileHash $downloadedClient).Hash.ToLowerInvariant()
    }
    $manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $manifestPath
    Install-CcpWindows stable '' $InstallDir
    Confirm-TestCondition ((Get-FileHash (Join-Path $InstallDir 'ccp-client.exe')).Hash -eq
        (Get-FileHash $downloadedClient).Hash) 'Re-running did not update the client.'
    $venvCalls = @($script:NativeCalls | Where-Object { $_.Arguments -contains 'venv' })
    Confirm-TestCondition ($venvCalls.Count -eq 2) 'Each successful upgrade must build a separate venv.'
    Test-CcpUpgradeRollback $fixture $InstallDir $manifest $manifestPath
    Test-CcpRestartableRecovery $fixture $InstallDir $manifest $manifestPath
    $aliasPath = Join-Path $InstallDir 'ccp.exe'
    if (Test-Path -LiteralPath $aliasPath) {
        Confirm-TestCondition ((Get-FileHash $aliasPath).Hash -eq (Get-FileHash $downloadedClient).Hash) 'Owned ccp alias was not updated.'
        [IO.File]::WriteAllText($aliasPath, 'unrelated ccp')
        Install-CcpWindows stable '' $InstallDir
        Confirm-TestCondition ((Get-Content -Encoding UTF8 -LiteralPath $aliasPath -Raw) -eq 'unrelated ccp') 'Existing ccp command was overwritten.'
    }
    $cursorPath = Join-Path $profileDirectory '.cursor/mcp.json'
    [IO.File]::WriteAllText($cursorPath, '{"mcpServers":{"ccp":{"command":"custom"}}}')
    Set-CcpJsonHost $cursorPath 'new' 'new'
    $cursor = Get-Content -Encoding UTF8 -LiteralPath $cursorPath -Raw | ConvertFrom-Json
    Confirm-TestCondition ($cursor.mcpServers.ccp.command -eq 'custom') 'Existing CCP host config was overwritten.'
    $outside = Join-Path $testRoot 'outside-profile'
    New-Item -ItemType Directory -Path $outside | Out-Null
    $redirected = Join-Path $profileDirectory 'redirected'
    $linkType = if ($savedEnvironment['OS'] -eq 'Windows_NT') { 'Junction' } else { 'SymbolicLink' }
    New-Item -ItemType $linkType -Path $redirected -Target $outside | Out-Null
    try {
        Confirm-TestFailure { Publish-CcpFile (Join-Path $InstallDir 'ccp-client.exe') (Join-Path $redirected 'client.exe') } 'Linked publication parent was accepted.'
        Confirm-TestFailure { Confirm-CcpPrivateStage $redirected } 'Linked staging directory was accepted.'
        Confirm-TestCondition (-not (Test-Path -LiteralPath (Join-Path $outside 'client.exe'))) 'Linked parent redirected publication outside the user profile.'
    } finally { (Get-Item -LiteralPath $redirected -Force).Delete() }
    $interruptedTarget = Join-Path $InstallDir 'interrupted.exe'
    [IO.File]::WriteAllText($interruptedTarget, 'retain old verified bytes')
    $script:SavedStageCheck = ${function:Confirm-CcpPrivateStage}
    $script:StageChecks = 0
    Set-Item Function:Confirm-CcpPrivateStage {
        param([string]$Path)
        $script:StageChecks++
        if ($script:StageChecks -eq 2) { throw 'Fixture interrupted before final publication.' }
        & $script:SavedStageCheck $Path
    }
    try {
        Confirm-TestFailure { Publish-CcpFile (Join-Path $InstallDir 'ccp-client.exe') $interruptedTarget } 'Interrupted publication was accepted.'
        Confirm-TestCondition ($script:StageChecks -ge 3) 'Final staging recheck or cleanup did not run.'
        Confirm-TestCondition ((Get-Content -Encoding UTF8 -LiteralPath $interruptedTarget -Raw) -eq 'retain old verified bytes') 'Interrupted publication replaced the old client.'
    } finally { Set-Item Function:Confirm-CcpPrivateStage $script:SavedStageCheck }
    if ($savedEnvironment['OS'] -eq 'Windows_NT') {
        $private = New-CcpPrivateStage $profileDirectory
        $originalAcl = Get-Acl -LiteralPath $private
        $expandedAcl = Get-Acl -LiteralPath $private
        $expandedAcl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
            [Security.Principal.SecurityIdentifier]::new('S-1-1-0'), 'Read', 'Allow'))
        Set-Acl -LiteralPath $private -AclObject $expandedAcl
        Confirm-TestFailure { Confirm-CcpPrivateStage $private } 'Overly broad staging ACL was accepted.'
        Set-Acl -LiteralPath $private -AclObject $originalAcl
        Remove-CcpPrivateStage $private
    }
    Test-CcpBoundedRetention $fixture $InstallDir $manifest $manifestPath
    $directoryTarget = Join-Path $InstallDir 'directory'
    New-Item -ItemType Directory -Path $directoryTarget | Out-Null
    Confirm-TestFailure { Publish-CcpFile (Join-Path $fixture $CcpClientAsset) $directoryTarget } 'Directory destination accepted.'
    Confirm-TestCondition (@(Get-ChildItem -LiteralPath $InstallDir -Force -Filter '.ccp-stage-*').Count -eq 0) 'Staging files leaked.'
    foreach ($url in $script:DownloadUrls) {
        Confirm-TestCondition ($url.StartsWith("https://github.com/$CcpRepository/releases/download/v0.2.0/")) 'Download used an unofficial or moving release URL.'
    }
    if ($savedEnvironment['OS'] -eq 'Windows_NT') {
        $savedPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        try {
            & $setUserPath $InstallDir
            & $setUserPath $InstallDir
            $entries = [Environment]::GetEnvironmentVariable('Path', 'User') -split ';'
            Confirm-TestCondition (@($entries | Where-Object { $_ -eq $InstallDir }).Count -eq 1) 'User PATH update was not idempotent.'
        } finally { [Environment]::SetEnvironmentVariable('Path', $savedPath, 'User') }
    }
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
    }
    if (Test-Path -LiteralPath $testRoot) { Remove-Item -LiteralPath $testRoot -Recurse -Force }
}
# Expected native failures leave LASTEXITCODE nonzero. Report success only after
# every assertion and cleanup completed; a terminating failure never reaches here.
Write-Information -InformationAction Continue 'Windows installer fixture regressions passed.'
exit 0
