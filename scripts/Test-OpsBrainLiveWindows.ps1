#requires -Version 7.4
# Operational-credential-free Windows runtime checks for the live installer and launchers.

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
fs.mkdirSync(`${configDirectory}/runtime-created`);
fs.writeFileSync(`${configDirectory}/runtime-created/state.json`, '{}');
fs.writeFileSync(capture, `${config}\n---CONFIG-DIR---\n${configDirectory}\n---ARGS---\n${args.join('\n')}`);
'@
    [IO.File]::WriteAllText($fakeClaudeScript, $source, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($fakeClaude, "@echo off`r`nnode `"%~dp0fake-claude.mjs`" %*`r`n", [Text.UTF8Encoding]::new($false))

    $fixtureToken = [guid]::NewGuid().ToString('N')
    $fixtureSecret = ConvertTo-SecureString $fixtureToken -AsPlainText -Force
    $claudeCredential = Join-Path $testDirectory 'claude.cred.xml'
    $codexCredential = Join-Path $testDirectory 'codex.cred.xml'
    [PSCredential]::new('CC-CI', $fixtureSecret) | Export-Clixml -LiteralPath $claudeCredential
    [PSCredential]::new('Codex-CI', $fixtureSecret) | Export-Clixml -LiteralPath $codexCredential
    $fixtureSecret = $null

    $validationOutput = & "$PSScriptRoot\read-ops-brain-agent-token.ps1" `
        -AgentCredentialFile $claudeCredential -AgentName 'CC-CI' -ValidateOnly
    Assert-True ($null -eq $validationOutput) 'credential identity preflight emitted output'
    $mismatchRejected = $false
    try {
        & "$PSScriptRoot\read-ops-brain-agent-token.ps1" `
            -AgentCredentialFile $claudeCredential -AgentName 'Codex-CI' -ValidateOnly
    }
    catch { $mismatchRejected = $_.Exception.Message -like '*Credential identity does not match*' }
    Assert-True $mismatchRejected 'credential identity preflight accepted a crossed identity'

    # A child-process capture is the supported operating-system-pipe path. Keep the
    # generated fixture in memory and never render it in test output.
    $pwsh = (@(Get-Command pwsh -CommandType Application -ErrorAction Stop) | Select-Object -First 1).Source
    $helperStartInfo = [Diagnostics.ProcessStartInfo]::new($pwsh)
    $helperStartInfo.UseShellExecute = $false
    $helperStartInfo.RedirectStandardOutput = $true
    $helperStartInfo.RedirectStandardError = $true
    foreach ($argument in @(
        '-NoLogo', '-NoProfile', '-NonInteractive', '-File', "$PSScriptRoot\read-ops-brain-agent-token.ps1",
        '-AgentCredentialFile', $claudeCredential, '-AgentName', 'CC-CI'
    )) { [void]$helperStartInfo.ArgumentList.Add($argument) }
    $helperProcess = [Diagnostics.Process]::Start($helperStartInfo)
    $helperOutputTask = $helperProcess.StandardOutput.ReadToEndAsync()
    $helperErrorTask = $helperProcess.StandardError.ReadToEndAsync()
    $helperProcess.WaitForExit()
    $helperOutput = $helperOutputTask.GetAwaiter().GetResult()
    $helperError = $helperErrorTask.GetAwaiter().GetResult()
    Assert-True ($helperProcess.ExitCode -eq 0) 'credential helper rejected its child-process pipe'
    Assert-True ($helperError.Length -eq 0) 'credential helper wrote an error on its supported pipe path'
    Assert-True ($helperOutput -ceq $fixtureToken) 'credential helper pipe output did not match the generated fixture'
    $helperOutput = $null

    $fileOutput = Join-Path $testDirectory 'helper-file-output.txt'
    $fileError = Join-Path $testDirectory 'helper-file-error.txt'
    $fileProcess = Start-Process -FilePath $pwsh -ArgumentList @(
        '-NoLogo', '-NoProfile', '-NonInteractive', '-File',
        ('"{0}"' -f "$PSScriptRoot\read-ops-brain-agent-token.ps1"),
        '-AgentCredentialFile', ('"{0}"' -f $claudeCredential), '-AgentName', 'CC-CI'
    ) -RedirectStandardOutput $fileOutput -RedirectStandardError $fileError -Wait -PassThru
    Assert-True ($fileProcess.ExitCode -ne 0) 'credential helper accepted file-backed stdout'
    Assert-True ((Get-Item -LiteralPath $fileOutput).Length -eq 0) 'credential helper wrote to file-backed stdout'
    $fileErrorText = Get-Content -LiteralPath $fileError -Raw
    Assert-True ($fileErrorText -like '*Refusing to emit an agent token*') 'credential helper file test failed for an unexpected reason'

    # Start-Process gives a Windows console application its own console by
    # default. The probe reports through a sentinel file so neither its fixture
    # token nor its expected error is copied into this test's output stream.
    $consoleProbe = Join-Path $testDirectory 'helper-console-probe.ps1'
    $consoleSentinel = Join-Path $testDirectory 'helper-console-result.txt'
    $consoleProbeSource = @'
param(
    [Parameter(Mandatory)][string]$Helper,
    [Parameter(Mandatory)][string]$Credential,
    [Parameter(Mandatory)][string]$Sentinel
)
try {
    & $Helper -AgentCredentialFile $Credential -AgentName 'CC-CI'
    [IO.File]::WriteAllText($Sentinel, 'accepted', [Text.UTF8Encoding]::new($false))
}
catch {
    [IO.File]::WriteAllText($Sentinel, $_.Exception.Message, [Text.UTF8Encoding]::new($false))
}
'@
    [IO.File]::WriteAllText($consoleProbe, $consoleProbeSource, [Text.UTF8Encoding]::new($false))
    $consoleProcess = Start-Process -FilePath $pwsh -ArgumentList @(
        '-NoLogo', '-NoProfile', '-NonInteractive', '-File', ('"{0}"' -f $consoleProbe),
        '-Helper', ('"{0}"' -f "$PSScriptRoot\read-ops-brain-agent-token.ps1"),
        '-Credential', ('"{0}"' -f $claudeCredential),
        '-Sentinel', ('"{0}"' -f $consoleSentinel)
    ) -Wait -PassThru
    Assert-True ($consoleProcess.ExitCode -eq 0) 'credential helper console probe process failed'
    Assert-True (Test-Path -LiteralPath $consoleSentinel -PathType Leaf) 'credential helper console probe did not report a result'
    $consoleResult = Get-Content -LiteralPath $consoleSentinel -Raw
    Assert-True ($consoleResult -like '*Refusing to emit an agent token*') 'credential helper accepted console stdout'

    $missingProfile = Join-Path $testDirectory 'crossed-identity-profile-must-not-exist.json'
    foreach ($launcher in @('ops-brain-claude-live.ps1', 'ops-brain-codex-live.ps1')) {
        $launcherMismatchRejected = $false
        try {
            & "$PSScriptRoot\$launcher" -Mode DryRun `
                -ProfileFile $missingProfile `
                -LiveUrl 'wss://ops-brain.example/live' `
                -AgentCredentialFile $claudeCredential `
                -AgentName 'Codex-CI'
        }
        catch { $launcherMismatchRejected = $_.Exception.Message -like '*identity preflight failed*' }
        Assert-True $launcherMismatchRejected "$launcher did not reject a crossed identity before client discovery"
    }

    $claudeProfile = Join-Path $testDirectory 'claude-profile.json'
    $codexProfile = Join-Path $testDirectory 'codex-profile.json'
    & node "$PSScriptRoot\ops-brain-client" configure claude `
        --live-url wss://ops-brain.example/live --agent CC-CI `
        --credential-file $claudeCredential --label claude-ci --profile $claudeProfile
    Assert-True ($LASTEXITCODE -eq 0) 'ops-brain-client failed to configure the Claude profile'
    & node "$PSScriptRoot\ops-brain-client" configure claude `
        --live-url wss://ops-brain.example/live --agent CC-CI `
        --credential-file $claudeCredential --label claude-ci --profile $claudeProfile
    Assert-True ($LASTEXITCODE -eq 0) 'ops-brain-client failed to replace its owned Claude profile'
    & node "$PSScriptRoot\ops-brain-client" configure codex `
        --live-url wss://ops-brain.example/live --agent Codex-CI `
        --credential-file $codexCredential --label codex-ci --app-server-port 4600 --profile $codexProfile
    Assert-True ($LASTEXITCODE -eq 0) 'ops-brain-client failed to configure the Codex profile'
    $profileStatus = & "$PSScriptRoot\ops-brain-claude-live.ps1" -Mode Status -ProfileFile $claudeProfile
    Assert-True (@($profileStatus) -contains 'agent: CC-CI') 'Claude launcher did not load its protected client profile'
    $codexProfileStatus = & "$PSScriptRoot\ops-brain-codex-live.ps1" -Mode Status -ProfileFile $codexProfile
    Assert-True (@($codexProfileStatus) -contains 'App Server: ws://127.0.0.1:4600') 'Codex launcher did not load its profile App Server port'
    $originalPath = $env:PATH
    $originalClaudeConfigDirectory = $env:CLAUDE_CONFIG_DIR
    $claudeBase = Join-Path $testDirectory 'claude-base'
    [IO.Directory]::CreateDirectory($claudeBase) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $claudeBase 'agents')) | Out-Null
    [IO.File]::WriteAllText(
        (Join-Path $claudeBase '.claude.json'),
        '{"mcpServers":{"existing":{"command":"cmd.exe","env":{"API_KEY":"must-not-be-copied"}}}}',
        [Text.UTF8Encoding]::new($false)
    )
    $env:PATH = "$fakeBin;$originalPath"
    $env:CLAUDE_CONFIG_DIR = $claudeBase
    $env:OPS_BRAIN_TEST_CAPTURE = $captureFile
    $env:OPS_BRAIN_AGENT_TOKEN = 'fixture-must-not-reach-claude'
    try {
        $startInfo = [Diagnostics.ProcessStartInfo]::new($pwsh)
        $startInfo.UseShellExecute = $false
        foreach ($argument in @(
            '-NoLogo', '-NoProfile', '-File', "$PSScriptRoot\ops-brain-claude-live.ps1",
            '-LiveUrl', 'wss://ops-brain.example/live',
            '-AgentCredentialFile', $claudeCredential,
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
        if ($null -eq $originalClaudeConfigDirectory) {
            Remove-Item Env:CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue
        }
        else { $env:CLAUDE_CONFIG_DIR = $originalClaudeConfigDirectory }
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
    Assert-True ($server.command -eq $pwsh) 'generated Claude MCP command is not the resolved PowerShell executable'
    Assert-True ($server.args -contains '-AgentName') 'generated Claude MCP config omitted AgentName'
    Assert-True ($server.args -contains 'CC-CI') 'generated Claude MCP config omitted the expected identity'
    Assert-True ($capture -notlike "*$fixtureToken*") 'credential contents leaked into Claude capture'
    $fixtureToken = $null
    Assert-True ($capture -notlike '*must-not-be-copied*') 'existing MCP credential was copied into the Claude overlay'
    $configDirectory = ($parts[0] -split "`n", 2)[0]
    Assert-True (-not (Test-Path -LiteralPath $configDirectory)) 'temporary Claude config overlay was not removed'
    $arguments = @($parts[1] -split "`n")
    $configIndex = [Array]::IndexOf($arguments, '--mcp-config')
    Assert-True ($configIndex -ge 0) 'Claude did not receive the existing user MCP config by path'
    Assert-True ($arguments[$configIndex + 1] -eq (Join-Path $claudeBase '.claude.json')) 'Claude received the wrong existing MCP config path'
    Assert-True ($arguments -contains '--dangerously-load-development-channels') 'Claude Channel opt-in flag is missing'
    $resumeIndex = [Array]::IndexOf($arguments, 'resume')
    Assert-True ($resumeIndex -ge 0) 'trailing client arguments did not reach $ClaudeArgs'
    Assert-True ($arguments -contains '--model' -and $arguments -contains 'fixture-model') 'trailing client flag and its value did not reach $ClaudeArgs'
    $channelIndex = [Array]::IndexOf($arguments, '--dangerously-load-development-channels')
    Assert-True ($resumeIndex -lt $channelIndex) 'client arguments were not passed ahead of the launcher-owned Claude flags'

    # Status mode proves the trailing-argument binding without executing the
    # native Codex binary: with PositionalBinding on, 'resume' lands in $LiveUrl
    # and Assert-LiveUrl throws instead of reaching $CodexArgs.
    $codexStatus = $null
    try { $codexStatus = & "$PSScriptRoot\ops-brain-codex-live.ps1" -Mode Status -Label 'codex-ci' resume --model fixture-model }
    catch { $codexStatus = @() }
    Assert-True (@($codexStatus) -contains 'label: codex-ci') 'Codex launcher mis-bound a trailing client argument instead of collecting it into $CodexArgs'

    [IO.File]::WriteAllText((Join-Path $fakeBin 'codex.cmd'), "@echo off`r`nexit /b 0`r`n", [Text.UTF8Encoding]::new($false))
    $originalAppData = $env:APPDATA
    $vendoredCodex = Join-Path $testDirectory 'appdata\npm\node_modules\@openai\codex\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe'
    [IO.Directory]::CreateDirectory((Split-Path -Parent $vendoredCodex)) | Out-Null
    [IO.File]::WriteAllText($vendoredCodex, 'fixture', [Text.UTF8Encoding]::new($false))
    try {
        $pwshDirectory = Split-Path -Parent $pwsh
        $nodeDirectory = Split-Path -Parent ((@(Get-Command node -CommandType Application -ErrorAction Stop) | Select-Object -First 1).Source)
        $env:PATH = "$fakeBin;$pwshDirectory;$nodeDirectory;$env:SystemRoot\System32;$env:SystemRoot"
        $env:APPDATA = Join-Path $testDirectory 'appdata'
        $installerStatus = & "$PSScriptRoot\Install-OpsBrainLive.ps1" -Mode Status -BinDirectory $binDirectory
        Assert-True (($installerStatus -join "`n") -like "*codex (native required): $vendoredCodex*") 'installer status did not resolve the npm package vendored native Codex binary'
        $codexDryRun = & "$PSScriptRoot\ops-brain-codex-live.ps1" -Mode DryRun `
            -LiveUrl 'wss://ops-brain.example/live' `
            -AgentCredentialFile $codexCredential `
            -AgentName 'Codex-CI'
        Assert-True (@($codexDryRun) -contains 'would launch one Codex TUI through that App Server') 'Codex launcher did not accept the vendored native binary'
    }
    finally {
        $env:PATH = $originalPath
        if ($null -eq $originalAppData) { Remove-Item Env:APPDATA -ErrorAction SilentlyContinue }
        else { $env:APPDATA = $originalAppData }
    }

    'Windows live launcher tests passed'
}
finally {
    $resolved = [IO.Path]::GetFullPath($testDirectory)
    if ($resolved.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase) -and
        [IO.Path]::GetFileName($resolved).StartsWith('ops-brain-live-windows-test-', [StringComparison]::Ordinal)) {
        [IO.Directory]::Delete($resolved, $true)
    }
}
