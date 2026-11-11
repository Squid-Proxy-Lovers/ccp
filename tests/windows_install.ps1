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
$engine = (Get-Process -Id $PID).Path
Invoke-CcpNative $engine @('-NoProfile', '-NonInteractive', '-Command', "[Console]::Error.WriteLine('fixture stderr'); exit 0")
$nativeFailed = $false
try { Invoke-CcpNative $engine @('-NoProfile', '-NonInteractive', '-Command', 'exit 5') }
catch { $nativeFailed = $true }
if (-not $nativeFailed) { throw 'Native failure exit code was ignored.' }
$missingFailed = $false
try { Invoke-CcpNative 'ccp-fixture-missing-command-4ccac7' @() }
catch { $missingFailed = $true }
if (-not $missingFailed) { throw 'Missing native verifier was accepted.' }
$CcpRepository = 'Squid-Proxy-Lovers/ccp'
$CcpClientAsset = 'ccp-client-windows-x86_64.exe'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) "ccp-installer-test-$([Guid]::NewGuid().ToString('N'))"
$fixture = Join-Path $testRoot 'fixture'
$profileDirectory = Join-Path $testRoot 'profile with spaces'
$InstallDir = Join-Path $profileDirectory 'bin'
$script:NativeCalls = [Collections.Generic.List[object]]::new()
$script:RejectAttestation = $false
$script:DownloadUrls = [Collections.Generic.List[string]]::new()
$savedEnvironment = @{}
foreach ($name in @('OS', 'PROCESSOR_ARCHITECTURE', 'PROCESSOR_ARCHITEW6432', 'USERPROFILE', 'LOCALAPPDATA', 'APPDATA', 'Path')) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

function Confirm-TestCondition {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}
function Confirm-TestFailure {
    param([scriptblock]$Action, [string]$Message)
    $failed = $false
    try { & $Action } catch { $failed = $true }
    Confirm-TestCondition $failed $Message
}
function Invoke-CcpNative {
    param([string]$Executable, [string[]]$ArgumentList)
    $script:NativeCalls.Add(@{ Executable = $Executable; Arguments = $ArgumentList })
    if ($Executable -eq 'gh' -and $script:RejectAttestation) { throw 'Fixture attestation failure.' }
    if ($ArgumentList -contains 'venv') {
        $scripts = Join-Path $ArgumentList[-1] 'Scripts'
        New-Item -ItemType Directory -Force -Path $scripts | Out-Null
        [IO.File]::WriteAllText((Join-Path $scripts 'python.exe'), 'fixture')
    }
    if ($ArgumentList -contains 'pip') {
        [IO.File]::WriteAllText((Join-Path (Split-Path $Executable) 'ccp-mcp-server.exe'), 'fixture')
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
    New-Item -ItemType Directory -Force -Path $fixture, $profileDirectory | Out-Null
    $artifacts = @{}
    foreach ($name in @($CcpClientAsset, 'ccp-mcp.tar.gz', 'build-info.json')) {
        $path = Join-Path $fixture $name
        [IO.File]::WriteAllText($path, "fixture bytes for $name")
        $artifacts[$name] = @{ size = (Get-Item $path).Length; sha256 = (Get-FileHash $path).Hash.ToLowerInvariant() }
    }
    $manifest = @{
        format = 1; repository = $CcpRepository; source_sha = ('a' * 40)
        package_version = '0.2.0'; platforms = @('windows-x86_64'); artifacts = $artifacts
    }
    $manifestPath = Join-Path $fixture 'ccp-release.json'
    $manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $manifestPath
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
    $script:RejectAttestation = $true
    Confirm-TestFailure { Install-CcpWindows stable '' $InstallDir } 'Full flow ignored attestation failure.'
    Confirm-TestCondition (-not (Test-Path -LiteralPath $InstallDir)) 'Failed verification created the install directory.'
    $script:RejectAttestation = $false
    Install-CcpWindows stable '' $InstallDir
    Confirm-TestCondition ($script:ObservedBinDirectory -eq $InstallDir) 'PATH was not configured.'
    Confirm-TestCondition (Test-Path -LiteralPath (Join-Path $InstallDir 'ccp-client.exe')) 'Client was not installed.'
    $claude = Get-Content -LiteralPath (Join-Path $profileDirectory '.claude.json') -Raw | ConvertFrom-Json
    Confirm-TestCondition ($claude.other -eq 'keep') 'Unrelated host setting was lost.'
    Confirm-TestCondition ($claude.mcpServers.ccp.env.CCP_CLIENT_BIN -eq (Join-Path $InstallDir 'ccp-client.exe')) 'MCP client path was not encoded correctly.'
    Confirm-TestCondition ($script:NativeCalls.Executable -contains 'codex') 'Codex MCP command was not called.'
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
    Confirm-TestCondition ($venvCalls.Count -eq 1) 'An update recreated the venv.'
    $aliasPath = Join-Path $InstallDir 'ccp.exe'
    if (Test-Path -LiteralPath $aliasPath) {
        Confirm-TestCondition ((Get-FileHash $aliasPath).Hash -eq (Get-FileHash $downloadedClient).Hash) 'Owned ccp alias was not updated.'
        [IO.File]::WriteAllText($aliasPath, 'unrelated ccp')
        Install-CcpWindows stable '' $InstallDir
        Confirm-TestCondition ((Get-Content -LiteralPath $aliasPath -Raw) -eq 'unrelated ccp') 'Existing ccp command was overwritten.'
    }
    $cursorPath = Join-Path $profileDirectory '.cursor/mcp.json'
    [IO.File]::WriteAllText($cursorPath, '{"mcpServers":{"ccp":{"command":"custom"}}}')
    Set-CcpJsonHost $cursorPath 'new' 'new'
    $cursor = Get-Content -LiteralPath $cursorPath -Raw | ConvertFrom-Json
    Confirm-TestCondition ($cursor.mcpServers.ccp.command -eq 'custom') 'Existing CCP host config was overwritten.'
    $directoryTarget = Join-Path $InstallDir 'directory'
    New-Item -ItemType Directory -Path $directoryTarget | Out-Null
    Confirm-TestFailure { Publish-CcpFile (Join-Path $fixture $CcpClientAsset) $directoryTarget } 'Directory destination accepted.'
    Confirm-TestCondition (@(Get-ChildItem -LiteralPath $InstallDir -Filter '*.stage.*').Count -eq 0) 'Staging files leaked.'
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
    Write-Information -InformationAction Continue 'Windows installer fixture regressions passed.'
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
    }
    if (Test-Path -LiteralPath $testRoot) { Remove-Item -LiteralPath $testRoot -Recurse -Force }
}
