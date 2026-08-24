#requires -Version 7.4
# Credential-free Windows runtime checks for the live installer and launchers.

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-True {
    param([Parameter(Mandatory)][bool]$Condition, [Parameter(Mandatory)][string]$Message)
    if (-not $Condition) { throw "FAIL: $Message" }
}

$RepoDirectory = Split-Path -Parent $PSScriptRoot
$temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$testDirectory = Join-Path $temporaryRoot "ops-brain-live-windows-test-$([guid]::NewGuid().ToString('N'))"
[IO.Directory]::CreateDirectory($testDirectory) | Out-Null

try {
    $parseFailed = $false
    Get-ChildItem $PSScriptRoot -Filter '*.ps1' | ForEach-Object {
        $tokens = $null
        $errors = $null
        [void][System.Management.Automation.Language.Parser]::ParseFile(
            $_.FullName,
            [ref]$tokens,
            [ref]$errors
        )
        if ($errors.Count -gt 0) {
            $parseFailed = $true
            $errors | ForEach-Object { Write-Error "$($_.Extent.File):$($_.Extent.StartLineNumber): $($_.Message)" }
        }
    }
    Assert-True (-not $parseFailed) 'one or more PowerShell launchers failed to parse'

    $binDirectory = Join-Path $testDirectory 'bin'
    & "$PSScriptRoot\Install-OpsBrain.ps1" -SkipDependencies -BinDirectory $binDirectory
    $installStatus = & "$PSScriptRoot\Install-OpsBrainLive.ps1" -Mode Status -BinDirectory $binDirectory
    Assert-True (@($installStatus) -contains 'claude adapter deps: ready') 'Windows installer did not verify bundled Claude dependencies'
    Assert-True (@($installStatus) -contains 'codex adapter deps: ready') 'Windows installer did not verify bundled Codex dependencies'
    Assert-True (Test-Path -LiteralPath (Join-Path $binDirectory 'ops-brain-client.cmd')) 'client profile/doctor shim is missing'
    Assert-True (Test-Path -LiteralPath (Join-Path $binDirectory 'ops-brain-claude.cmd')) 'Claude command shim is missing'
    Assert-True (Test-Path -LiteralPath (Join-Path $binDirectory 'ops-brain-codex.cmd')) 'Codex command shim is missing'
    & "$PSScriptRoot\ops-brain-claude-live.ps1" -Mode Status
    $codexStatusOutput = & "$PSScriptRoot\ops-brain-codex-live.ps1" -Mode Status
    Assert-True (@($codexStatusOutput) -contains 'App Server: ws://127.0.0.1:4500') 'Codex launcher status did not preserve the bare App Server endpoint'

    $unsafeBin = Join-Path $testDirectory 'unsafe-bin'
    [IO.Directory]::CreateDirectory($unsafeBin) | Out-Null
    $ownedShim = Join-Path $unsafeBin 'ops-brain-codex-live.cmd'
    [IO.File]::WriteAllText($ownedShim, "unrelated content`r`n", [Text.UTF8Encoding]::new($false))
    $refused = $false
    try { & "$PSScriptRoot\Install-OpsBrainLive.ps1" -SkipDependencies -BinDirectory $unsafeBin }
    catch { $refused = $_.Exception.Message -like '*Refusing to replace non-owned launcher*' }
    Assert-True $refused 'installer overwrote or accepted a non-owned shim'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $unsafeBin 'ops-brain-claude-live.cmd'))) 'installer partially created the first shim before preflight failed'

    foreach ($badUrl in @(
        'wss://user:do-not-print@ops.example/live',
        'wss://ops.example/live?token=do-not-print',
        'wss://ops.example/mcp',
        'ws://ops.example/live'
    )) {
        $rejected = $false
        try {
            & "$PSScriptRoot\ops-brain-claude-live.ps1" -Mode Status -LiveUrl $badUrl
        }
        catch { $rejected = $true }
        Assert-True $rejected "Claude launcher accepted invalid URL shape: $badUrl"
        $rejected = $false
        try {
            & "$PSScriptRoot\ops-brain-codex-live.ps1" -Mode Status -LiveUrl $badUrl
        }
        catch { $rejected = $true }
        Assert-True $rejected "Codex launcher accepted invalid URL shape: $badUrl"
    }

    $fakeBin = Join-Path $testDirectory 'fake-bin'
    [IO.Directory]::CreateDirectory($fakeBin) | Out-Null
    $captureFile = Join-Path $testDirectory 'claude-capture.txt'
    $fakeClaudeScript = Join-Path $fakeBin 'fake-claude.mjs'
    $fakeClaude = Join-Path $fakeBin 'claude.cmd'
    $source = @'
import fs from 'node:fs';
if (process.env.OPS_BRAIN_AGENT_TOKEN) process.exit(3);
const args = process.argv.slice(2);
const capture = process.env.OPS_BRAIN_TEST_CAPTURE;
const configDirectory = process.env.CLAUDE_CONFIG_DIR;
if (!capture || !configDirectory) process.exit(2);
const config = fs.readFileSync(`${configDirectory}/.claude.json`, 'utf8');
fs.writeFileSync(capture, `${config}\n---CONFIG-DIR---\n${configDirectory}\n---ARGS---\n${args.join('\n')}`);
'@
    [IO.File]::WriteAllText($fakeClaudeScript, $source, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($fakeClaude, "@echo off`r`nnode `"%~dp0fake-claude.mjs`" %*`r`n", [Text.UTF8Encoding]::new($false))

    $credential = Join-Path $testDirectory 'credential.cred.xml'
    [IO.File]::WriteAllText($credential, 'credential fixture is not read by the parent launcher')
    $claudeProfile = Join-Path $testDirectory 'claude-profile.json'
    $codexProfile = Join-Path $testDirectory 'codex-profile.json'
    & node "$PSScriptRoot\ops-brain-client" configure claude `
        --live-url wss://ops-brain.example/live --agent CC-CI `
        --credential-file $credential --label claude-ci --profile $claudeProfile
    Assert-True ($LASTEXITCODE -eq 0) 'ops-brain-client failed to configure the Claude profile'
    & node "$PSScriptRoot\ops-brain-client" configure claude `
        --live-url wss://ops-brain.example/live --agent CC-CI `
        --credential-file $credential --label claude-ci --profile $claudeProfile
    Assert-True ($LASTEXITCODE -eq 0) 'ops-brain-client failed to replace its owned Claude profile'
    & node "$PSScriptRoot\ops-brain-client" configure codex `
        --live-url wss://ops-brain.example/live --agent Codex-CI `
        --credential-file $credential --label codex-ci --app-server-port 4600 --profile $codexProfile
    Assert-True ($LASTEXITCODE -eq 0) 'ops-brain-client failed to configure the Codex profile'
    $profileStatus = & "$PSScriptRoot\ops-brain-claude-live.ps1" -Mode Status -ProfileFile $claudeProfile
    Assert-True (@($profileStatus) -contains 'agent: CC-CI') 'Claude launcher did not load its protected client profile'
    $codexProfileStatus = & "$PSScriptRoot\ops-brain-codex-live.ps1" -Mode Status -ProfileFile $codexProfile
    Assert-True (@($codexProfileStatus) -contains 'App Server: ws://127.0.0.1:4600') 'Codex launcher did not load its profile App Server port'
    $originalPath = $env:PATH
    $env:PATH = "$fakeBin;$originalPath"
    $env:OPS_BRAIN_TEST_CAPTURE = $captureFile
    $env:OPS_BRAIN_AGENT_TOKEN = 'fixture-must-not-reach-claude'
    try {
        $pwsh = (@(Get-Command pwsh -CommandType Application -ErrorAction Stop) | Select-Object -First 1).Source
        $startInfo = [Diagnostics.ProcessStartInfo]::new($pwsh)
        $startInfo.UseShellExecute = $false
        foreach ($argument in @(
            '-NoLogo', '-NoProfile', '-File', "$PSScriptRoot\ops-brain-claude-live.ps1",
            '-LiveUrl', 'wss://ops-brain.example/live',
            '-AgentCredentialFile', $credential,
            '-AgentName', 'CC-CI',
            '-Label', 'claude-ci',
            # Trailing client arguments. These must reach $ClaudeArgs; with
            # PositionalBinding left on, 'resume' binds to $Mode instead and the
            # launcher dies in ValidateSet before Claude is ever invoked.
            'resume', '--model', 'fixture-model'
        )) { [void]$startInfo.ArgumentList.Add($argument) }
        $process = [Diagnostics.Process]::Start($startInfo)
        $process.WaitForExit()
        Assert-True ($process.ExitCode -eq 0) "Claude launcher child exited $($process.ExitCode)"
    }
    finally {
        $env:PATH = $originalPath
        Remove-Item Env:OPS_BRAIN_TEST_CAPTURE -ErrorAction SilentlyContinue
        Remove-Item Env:OPS_BRAIN_AGENT_TOKEN -ErrorAction SilentlyContinue
    }

    $capture = [IO.File]::ReadAllText($captureFile)
    $configParts = $capture -split "`n---CONFIG-DIR---`n", 2
    Assert-True ($configParts.Count -eq 2) 'fake Claude did not receive an isolated config directory'
    $parts = $configParts[1] -split "`n---ARGS---`n", 2
    Assert-True ($parts.Count -eq 2) 'fake Claude did not capture arguments'
    $config = $configParts[0] | ConvertFrom-Json
    $server = $config.mcpServers.'ops-brain-live'
    Assert-True ($server.command -eq 'pwsh.exe') 'generated Claude MCP command is wrong'
    Assert-True ($server.args -contains '-AgentName') 'generated Claude MCP config omitted AgentName'
    Assert-True ($server.args -contains 'CC-CI') 'generated Claude MCP config omitted the expected identity'
    Assert-True ($capture -notlike '*credential fixture is not read*') 'credential contents leaked into Claude capture'
    $configDirectory = ($parts[0] -split "`n", 2)[0]
    Assert-True (-not (Test-Path -LiteralPath $configDirectory)) 'temporary Claude config overlay was not removed'
    $arguments = @($parts[1] -split "`n")
    Assert-True (-not ($arguments -contains '--mcp-config')) 'Claude still received the resolver-invisible --mcp-config path'
    Assert-True ($arguments -contains '--dangerously-load-development-channels') 'Claude Channel opt-in flag is missing'
    $resumeIndex = [Array]::IndexOf($arguments, 'resume')
    Assert-True ($resumeIndex -ge 0) 'trailing client arguments did not reach $ClaudeArgs'
    Assert-True ($arguments -contains '--model' -and $arguments -contains 'fixture-model') 'trailing client flag and its value did not reach $ClaudeArgs'
    $channelIndex = [Array]::IndexOf($arguments, '--dangerously-load-development-channels')
    Assert-True ($resumeIndex -lt $channelIndex) 'client arguments were not passed ahead of the launcher-owned Claude flags'

    # The Codex launcher has no credential-free end-to-end path (it refuses .cmd
    # shims, so there is no fake codex.exe to capture arguments). Status mode still
    # proves the binding: with PositionalBinding on, 'resume' lands in $LiveUrl and
    # Assert-LiveUrl throws on the relative URI instead of reaching $CodexArgs.
    $codexStatus = $null
    try { $codexStatus = & "$PSScriptRoot\ops-brain-codex-live.ps1" -Mode Status -Label 'codex-ci' resume --model fixture-model }
    catch { $codexStatus = @() }
    Assert-True (@($codexStatus) -contains 'label: codex-ci') 'Codex launcher mis-bound a trailing client argument instead of collecting it into $CodexArgs'

    [IO.File]::WriteAllText((Join-Path $fakeBin 'codex.cmd'), "@echo off`r`nexit /b 0`r`n", [Text.UTF8Encoding]::new($false))
    $shimRejected = $false
    try {
        $env:PATH = "$fakeBin;$env:SystemRoot\System32;$env:SystemRoot"
        $installerStatus = & "$PSScriptRoot\Install-OpsBrainLive.ps1" -Mode Status -BinDirectory $binDirectory
        Assert-True (($installerStatus -join "`n") -like '*codex (native required): <missing>*') 'installer status treated codex.cmd as a runnable native Codex binary'
        & "$PSScriptRoot\ops-brain-codex-live.ps1" -Mode DryRun `
            -LiveUrl 'wss://ops-brain.example/live' `
            -AgentCredentialFile $credential `
            -AgentName 'Codex-CI'
    }
    catch { $shimRejected = $_.Exception.Message -like '*Required executable is missing: codex.exe*' }
    finally { $env:PATH = $originalPath }
    Assert-True $shimRejected 'Codex launcher accepted a .cmd shim that Start-Process cannot own with redirected logs'

    'Windows live launcher tests passed'
}
finally {
    $resolved = [IO.Path]::GetFullPath($testDirectory)
    if ($resolved.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase) -and
        [IO.Path]::GetFileName($resolved).StartsWith('ops-brain-live-windows-test-', [StringComparison]::Ordinal)) {
        [IO.Directory]::Delete($resolved, $true)
    }
}
