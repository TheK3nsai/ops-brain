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
    Assert-True (@($installStatus) -contains "shell init: $PSScriptRoot\OpsBrain-Shell.ps1") 'Windows installer status does not point at the profile integration'
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
        '{"theme":"dark","hasCompletedOnboarding":true,"oauthAccount":{"emailAddress":"account-metadata-must-stay-private@example.test"},"mcpServers":{"existing":{"command":"cmd.exe","env":{"API_KEY":"must-not-be-copied"}}}}',
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
            '-StateDirectory', (Join-Path $testDirectory 'run-state'),
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
    Assert-True ($config.theme -eq 'dark') 'generated Claude overlay omitted the allowlisted theme'
    Assert-True ($config.hasCompletedOnboarding -eq $true) 'generated Claude overlay omitted completed onboarding state'
    $server = $config.mcpServers.'ops-brain-live'
    Assert-True ($server.command -eq $pwsh) 'generated Claude MCP command is not the resolved PowerShell executable'
    Assert-True ($server.args -contains '-AgentName') 'generated Claude MCP config omitted AgentName'
    Assert-True ($server.args -contains 'CC-CI') 'generated Claude MCP config omitted the expected identity'
    Assert-True ($capture -notlike "*$fixtureToken*") 'credential contents leaked into Claude capture'
    $fixtureToken = $null
    Assert-True ($capture -notlike '*must-not-be-copied*') 'existing MCP credential was copied into the Claude overlay'
    Assert-True ($capture -notlike '*account-metadata-must-stay-private@example.test*') 'existing account metadata was copied into the Claude overlay'
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

    # --- main-launcher (Auto) mode ------------------------------------------
    # A plain recorder for the passthrough paths: it must run with no overlay,
    # no Channel flag, and no bearer, exactly as if the launcher were absent.
    $plainBin = Join-Path $testDirectory 'plain-bin'
    [IO.Directory]::CreateDirectory($plainBin) | Out-Null
    [IO.File]::WriteAllText((Join-Path $plainBin 'plain-client.mjs'), @'
import fs from 'node:fs';
if (process.env.OPS_BRAIN_AGENT_TOKEN) process.exit(42);
const capture = process.env.OPS_BRAIN_TEST_CAPTURE;
if (!capture) process.exit(2);
fs.writeFileSync(`${capture}.args`, process.argv.slice(2).join('\n'));
fs.writeFileSync(`${capture}.dir`, process.env.CLAUDE_CONFIG_DIR ?? '<unset>');
'@, [Text.UTF8Encoding]::new($false))
    foreach ($client in @('claude', 'codex')) {
        [IO.File]::WriteAllText((Join-Path $plainBin "$client.cmd"), "@echo off`r`nnode `"%~dp0plain-client.mjs`" %*`r`n", [Text.UTF8Encoding]::new($false))
    }
    $claudeLauncher = "$PSScriptRoot\ops-brain-claude-live.ps1"
    $codexLauncher = "$PSScriptRoot\ops-brain-codex-live.ps1"
    $shellInit = "$PSScriptRoot\OpsBrain-Shell.ps1"
    $autoState = Join-Path $testDirectory 'auto-state'

    function ConvertTo-LiteralArgument {
        param($Value)
        if ($Value -is [array]) {
            return '@(' + (@($Value | ForEach-Object { "'" + ([string]$_).Replace("'", "''") + "'" }) -join ', ') + ')'
        }
        "'" + ([string]$Value).Replace("'", "''") + "'"
    }

    # A child pwsh whose stdin, stdout and stderr are all pipes: the headless
    # shape every wake shim, scheduled task and script has.
    function Invoke-PwshChild {
        param(
            [Parameter(Mandatory)][string[]]$Arguments,
            [hashtable]$Environment = @{},
            [string]$PathPrepend = $plainBin,
            [string]$WorkingDirectory = $PSScriptRoot
        )
        $startInfo = [Diagnostics.ProcessStartInfo]::new($pwsh)
        $startInfo.UseShellExecute = $false
        $startInfo.RedirectStandardInput = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $startInfo.WorkingDirectory = $WorkingDirectory
        foreach ($argument in @('-NoLogo', '-NoProfile') + $Arguments) { [void]$startInfo.ArgumentList.Add($argument) }
        $startInfo.Environment['PATH'] = "$PathPrepend;$($env:PATH)"
        foreach ($name in $Environment.Keys) { $startInfo.Environment[$name] = $Environment[$name] }
        $process = [Diagnostics.Process]::Start($startInfo)
        $process.StandardInput.Close()
        $outputTask = $process.StandardOutput.ReadToEndAsync()
        $errorTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(120000)) { $process.Kill($true); throw "child pwsh timed out: $($Arguments -join ' ')" }
        [pscustomobject]@{
            ExitCode = $process.ExitCode
            StdOut = $outputTask.GetAwaiter().GetResult()
            StdErr = $errorTask.GetAwaiter().GetResult()
        }
    }

    # Runs a launcher the way the profile function does: in-process, with the
    # client arguments as one array. Trailing tokens cannot carry -p through
    # pwsh -File (it binds to -ProfileFile by prefix), so this is the shape
    # under test.
    function Invoke-LauncherChild {
        param(
            [Parameter(Mandatory)][string]$Launcher,
            [Parameter(Mandatory)][hashtable]$Parameters,
            [hashtable]$Environment = @{},
            [string]$PathPrepend = $plainBin
        )
        $invocation = "& '" + $Launcher.Replace("'", "''") + "'"
        foreach ($name in $Parameters.Keys) {
            if ($Parameters[$name] -is [bool]) { $invocation += " -${name}:`$$($Parameters[$name].ToString().ToLowerInvariant())" }
            else { $invocation += " -$name " + (ConvertTo-LiteralArgument $Parameters[$name]) }
        }
        $invocation += '; exit $LASTEXITCODE'
        Invoke-PwshChild -Arguments @('-Command', $invocation) -Environment $Environment -PathPrepend $PathPrepend
    }

    # This child has no console, so Auto must pass through untouched even when
    # live is fully configured: a headless or piped session cannot render a
    # channel event and must never register a peer.
    $capture = Join-Path $testDirectory 'auto-notty'
    $result = Invoke-LauncherChild $claudeLauncher @{ Mode = 'Auto'; ProfileFile = $claudeProfile; StateDirectory = $autoState; ClaudeArgs = @('--resume', 'abc') } @{ OPS_BRAIN_TEST_CAPTURE = $capture; CLAUDE_CONFIG_DIR = $claudeBase }
    Assert-True ($result.ExitCode -eq 0) "headless Auto passthrough exited $($result.ExitCode): $($result.StdErr)"
    $captured = [IO.File]::ReadAllText("$capture.args")
    Assert-True ($captured -eq "--resume`nabc") 'headless Auto passthrough did not hand the client arguments through verbatim'
    Assert-True ([IO.File]::ReadAllText("$capture.dir") -eq $claudeBase) 'Auto passthrough replaced CLAUDE_CONFIG_DIR'
    Assert-True (($result.StdOut + $result.StdErr).Trim().Length -eq 0) "Auto passthrough should be silent, got: $($result.StdOut)$($result.StdErr)"
    Assert-True (-not (Test-Path -LiteralPath $autoState)) 'Auto passthrough created the adapter state directory'

    # Headless with a broken preflight is still a silent passthrough: a
    # wake shim must never be asked a question.
    $capture = Join-Path $testDirectory 'auto-notty-unconfigured'
    $result = Invoke-LauncherChild $claudeLauncher @{ Mode = 'Auto'; ProfileFile = $missingProfile; ClaudeArgs = @() } @{ OPS_BRAIN_TEST_CAPTURE = $capture }
    Assert-True ($result.ExitCode -eq 0 -and (Test-Path -LiteralPath "$capture.args")) 'headless Auto did not pass through on a broken preflight'
    Assert-True ($result.StdErr -notlike '*Continue without live delivery*') 'headless Auto prompted'

    # -p / --print and --version pass through with the prompt intact; the
    # array route is what keeps -p from binding to -ProfileFile.
    foreach ($case in @(
        @{ Name = 'auto-print'; Launcher = $claudeLauncher; Parameter = 'ClaudeArgs'; Arguments = @('-p', 'hello world') },
        @{ Name = 'auto-version'; Launcher = $claudeLauncher; Parameter = 'ClaudeArgs'; Arguments = @('--version') },
        @{ Name = 'codex-exec'; Launcher = $codexLauncher; Parameter = 'CodexArgs'; Arguments = @('exec', 'do the thing') },
        @{ Name = 'codex-version'; Launcher = $codexLauncher; Parameter = 'CodexArgs'; Arguments = @('--version') }
    )) {
        $capture = Join-Path $testDirectory $case.Name
        $parameters = @{ Mode = 'Auto'; ProfileFile = $claudeProfile }
        $parameters[$case.Parameter] = $case.Arguments
        $result = Invoke-LauncherChild $case.Launcher $parameters @{ OPS_BRAIN_TEST_CAPTURE = $capture }
        Assert-True ($result.ExitCode -eq 0) "$($case.Name) passthrough exited $($result.ExitCode): $($result.StdErr)"
        Assert-True ([IO.File]::ReadAllText("$capture.args") -eq ($case.Arguments -join "`n")) "$($case.Name) passthrough altered the client arguments"
        Assert-True ($result.StdErr.Trim().Length -eq 0) "$($case.Name) passthrough was not silent: $($result.StdErr)"
    }

    # --no-live and OPS_BRAIN_LIVE=off are the announced, deliberate ordinary
    # launches; they must say so once and pass every argument through.
    $capture = Join-Path $testDirectory 'no-live'
    $result = Invoke-LauncherChild $claudeLauncher @{ Mode = 'Auto'; ProfileFile = $claudeProfile; ClaudeArgs = @('--no-live', '-p', 'hello world') } @{ OPS_BRAIN_TEST_CAPTURE = $capture }
    Assert-True ($result.StdErr -like '*ops-brain live: off (requested)*') '--no-live was not announced'
    Assert-True ([IO.File]::ReadAllText("$capture.args") -eq "-p`nhello world") '--no-live did not hand the remaining arguments through'
    $capture = Join-Path $testDirectory 'env-off'
    $result = Invoke-LauncherChild $codexLauncher @{ Mode = 'Auto'; ProfileFile = $codexProfile; CodexArgs = @() } @{ OPS_BRAIN_TEST_CAPTURE = $capture; OPS_BRAIN_LIVE = 'off' }
    Assert-True ($result.StdErr -like '*ops-brain live: off (requested)*') 'OPS_BRAIN_LIVE=off was not announced'
    Assert-True (Test-Path -LiteralPath "$capture.args") 'OPS_BRAIN_LIVE=off did not reach the real codex'
    $capture = Join-Path $testDirectory 'switch-off'
    $result = Invoke-LauncherChild $claudeLauncher @{ Mode = 'Run'; NoLive = $true; ProfileFile = $missingProfile; ClaudeArgs = @('--version') } @{ OPS_BRAIN_TEST_CAPTURE = $capture }
    Assert-True ($result.StdErr -like '*ops-brain live: off (requested)*' -and (Test-Path -LiteralPath "$capture.args")) '-NoLive did not launch an announced ordinary session'

    # Without Auto the explicit command keeps failing closed: no prompt, no
    # ordinary fallback, nonzero exit.
    $capture = Join-Path $testDirectory 'explicit'
    $result = Invoke-LauncherChild $claudeLauncher @{ Mode = 'Run'; ProfileFile = $missingProfile; LiveUrl = 'wss://ops-brain.example/live' } @{ OPS_BRAIN_TEST_CAPTURE = $capture }
    Assert-True ($result.ExitCode -ne 0) 'explicit launcher launched ordinary Claude on a failed preflight'
    Assert-True (-not (Test-Path -LiteralPath "$capture.args")) 'explicit launcher fell back to ordinary Claude'
    Assert-True ($result.StdErr -like '*AgentName is required*') "explicit launcher failed for an unexpected reason: $($result.StdErr)"
    Assert-True ($result.StdErr -notlike '*Continue without live delivery*') 'explicit launcher prompted'
    $result = Invoke-LauncherChild $codexLauncher @{ Mode = 'Run'; ProfileFile = $missingProfile; LiveUrl = 'wss://ops-brain.example/live' } @{ OPS_BRAIN_TEST_CAPTURE = $capture }
    Assert-True ($result.ExitCode -ne 0 -and -not (Test-Path -LiteralPath "$capture.args") -and $result.StdErr -notlike '*Continue without live delivery*') 'explicit Codex launcher did not fail closed'

    # The label carries the working directory so sibling sessions of one identity
    # can be told apart; unsafe characters are folded and the length is bounded.
    $workRoot = Join-Path $testDirectory 'work'
    $spacedDirectory = Join-Path $workRoot 'ops brain (v2)'
    $longDirectory = Join-Path $workRoot ('d' * 90)
    [IO.Directory]::CreateDirectory($spacedDirectory) | Out-Null
    [IO.Directory]::CreateDirectory($longDirectory) | Out-Null
    $env:PATH = "$plainBin;$originalPath"
    $env:APPDATA = Join-Path $testDirectory 'appdata'
    try {
        Push-Location -LiteralPath $spacedDirectory
        try {
            $labelDry = @(& $claudeLauncher -Mode DryRun -ProfileFile $claudeProfile)
            $codexLabelDry = @(& $codexLauncher -Mode DryRun -ProfileFile $codexProfile)
        }
        finally { Pop-Location }
        Assert-True ($labelDry -contains 'label: claude-ci.ops-brain--v2-') "Claude dry run did not fold the working directory into the label: $($labelDry -join '; ')"
        Assert-True ($codexLabelDry -contains 'label: codex-ci.ops-brain--v2-') "Codex dry run did not fold the working directory into the label: $($codexLabelDry -join '; ')"
        Push-Location -LiteralPath $longDirectory
        try { $longLabel = [string](@(& $claudeLauncher -Mode DryRun -ProfileFile $claudeProfile) | Where-Object { $_ -like 'label: *' } | Select-Object -First 1) }
        finally { Pop-Location }
        $longLabel = $longLabel.Substring('label: '.Length)
        Assert-True ([Text.Encoding]::UTF8.GetByteCount($longLabel) -eq 80) "label was not bounded to 80 bytes: $($longLabel.Length)"
        Assert-True ($longLabel -cmatch '^[A-Za-z0-9._-]{80}$') 'bounded label left the label alphabet'
    }
    finally {
        $env:PATH = $originalPath
        if ($null -eq $originalAppData) { Remove-Item Env:APPDATA -ErrorAction SilentlyContinue }
        else { $env:APPDATA = $originalAppData }
    }

    # The profile integration defines functions only in an attended console
    # session and carries no credential material.
    Assert-True (Test-Path -LiteralPath $shellInit -PathType Leaf) 'profile integration file missing'
    $shellCode = @(Get-Content -LiteralPath $shellInit) | Where-Object { $_ -notmatch '^\s*#' }
    Assert-True (-not ($shellCode -match 'OPS_BRAIN_AGENT_TOKEN|Authorization|\$env:')) 'profile integration touches credentials or the environment'
    $functionCount = ". '$shellInit'; @(Get-Command claude, codex -CommandType Function -ErrorAction SilentlyContinue).Count"
    foreach ($switches in @(@('-NonInteractive'), @())) {
        $result = Invoke-PwshChild -Arguments ($switches + @('-Command', $functionCount))
        Assert-True ($result.ExitCode -eq 0 -and $result.StdOut.Trim() -eq '0') "profile integration defined functions in a non-interactive shell ($($switches -join ' ')): $($result.StdOut)$($result.StdErr)"
    }

    # The attended paths need a real console: a hidden child console whose
    # input buffer is fed through WriteConsoleInput, so the launcher's
    # redirection checks and Read-Host see exactly what an operator's terminal
    # provides. Each case runs the launcher in-process, as the profile
    # function does, and reports through a file.
    $consoleProbe = Join-Path $testDirectory 'auto-console-probe.ps1'
    [IO.File]::WriteAllText($consoleProbe, @'
param([Parameter(Mandatory)][string]$CaseFile)
$case = Get-Content -LiteralPath $CaseFile -Raw | ConvertFrom-Json -AsHashtable
$result = @{ attended = (-not [Console]::IsInputRedirected -and -not [Console]::IsOutputRedirected); exit = $null; stderr = ''; error = ''; functions = -1 }
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class OpsBrainTestConsoleInput {
    [StructLayout(LayoutKind.Sequential)]
    public struct KEY_EVENT_RECORD { public int bKeyDown; public short wRepeatCount; public short wVirtualKeyCode; public short wVirtualScanCode; public char UnicodeChar; public int dwControlKeyState; }
    [StructLayout(LayoutKind.Explicit)]
    public struct INPUT_RECORD { [FieldOffset(0)] public short EventType; [FieldOffset(4)] public KEY_EVENT_RECORD KeyEvent; }
    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr GetStdHandle(int nStdHandle);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool WriteConsoleInputW(IntPtr hConsoleInput, INPUT_RECORD[] lpBuffer, int nLength, out int lpNumberOfEventsWritten);
    public static int Type(string text) {
        var records = new INPUT_RECORD[text.Length * 2];
        int i = 0;
        foreach (char c in text) {
            short vk = c == '\r' ? (short)0x0D : (short)0;
            records[i++] = new INPUT_RECORD { EventType = 1, KeyEvent = new KEY_EVENT_RECORD { bKeyDown = 1, wRepeatCount = 1, wVirtualKeyCode = vk, UnicodeChar = c } };
            records[i++] = new INPUT_RECORD { EventType = 1, KeyEvent = new KEY_EVENT_RECORD { bKeyDown = 0, wRepeatCount = 1, wVirtualKeyCode = vk, UnicodeChar = c } };
        }
        int written;
        if (!WriteConsoleInputW(GetStdHandle(-10), records, records.Length, out written)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        return written;
    }
}
"@
$writer = [IO.StringWriter]::new()
try {
    if ($case.pathPrepend) { $env:PATH = "$($case.pathPrepend);$env:PATH" }
    foreach ($name in $case.env.Keys) { Set-Item -Path "Env:$name" -Value $case.env[$name] }
    if ($case.workingDirectory) { Set-Location -LiteralPath $case.workingDirectory }
    if ($case.typed -and $result.attended) { [void][OpsBrainTestConsoleInput]::Type($case.typed) }
    [Console]::SetError($writer)
    if ($case.shellInit) {
        . $case.shellInit
        $result.functions = @(Get-Command claude, codex -CommandType Function -ErrorAction SilentlyContinue).Count
        $arguments = @($case.arguments)
        & $case.command @arguments
    }
    else {
        $parameters = $case.parameters
        & $case.launcher @parameters
    }
    $result.exit = $LASTEXITCODE
}
catch {
    $result.error = $_.Exception.Message
    if ($null -eq $result.exit) { $result.exit = 1 }
}
finally {
    $result.stderr = $writer.ToString()
    [IO.File]::WriteAllText($case.result, ($result | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new($false))
}
'@, [Text.UTF8Encoding]::new($false))

    function Invoke-ConsoleProbe {
        param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][hashtable]$Case)
        $caseFile = Join-Path $testDirectory "$Name.case.json"
        $resultFile = Join-Path $testDirectory "$Name.result.json"
        $Case['result'] = $resultFile
        if (-not $Case.ContainsKey('pathPrepend')) { $Case['pathPrepend'] = $plainBin }
        if (-not $Case.ContainsKey('env')) { $Case['env'] = @{} }
        if (-not $Case.ContainsKey('workingDirectory')) { $Case['workingDirectory'] = $PSScriptRoot }
        [IO.File]::WriteAllText($caseFile, ($Case | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
        $process = Start-Process -FilePath $pwsh -ArgumentList @('-NoLogo', '-NoProfile', '-File', ('"{0}"' -f $consoleProbe), '-CaseFile', ('"{0}"' -f $caseFile)) -WindowStyle Hidden -PassThru
        if (-not $process.WaitForExit(120000)) { $process.Kill($true); throw "console probe $Name did not finish (a prompt with no answer?)" }
        Assert-True (Test-Path -LiteralPath $resultFile -PathType Leaf) "console probe $Name reported nothing"
        Get-Content -LiteralPath $resultFile -Raw | ConvertFrom-Json
    }

    # Capability check doubles as the shell-function test: a real console
    # defines claude/codex, and the function's `claude --version` is a silent
    # passthrough to the plain recorder.
    $capture = Join-Path $testDirectory 'console-function'
    $probe = Invoke-ConsoleProbe 'console-function' @{ shellInit = $shellInit; command = 'claude'; arguments = @('--version'); env = @{ OPS_BRAIN_TEST_CAPTURE = $capture } }
    if (-not $probe.attended) {
        Write-Warning 'note: no attended console could be allocated here; the Auto prompt and attended passthrough paths were not exercised'
    }
    else {
        Assert-True ($probe.functions -eq 2) "profile integration defined $($probe.functions) functions in an attended console"
        Assert-True ($probe.exit -eq 0 -and $probe.error -eq '') "shell function launch failed: $($probe.error)"
        Assert-True ([IO.File]::ReadAllText("$capture.args") -eq '--version') 'shell function did not pass --version through'
        Assert-True ($probe.stderr.Trim().Length -eq 0) "attended --version passthrough was not silent: $($probe.stderr)"

        # Attended, configured: Auto goes live through the overlay exactly like
        # the explicit command, and announces the identity and label.
        $capture = Join-Path $testDirectory 'console-live.txt'
        $probe = Invoke-ConsoleProbe 'console-live' @{
            launcher = $claudeLauncher; pathPrepend = $fakeBin
            parameters = @{ Mode = 'Auto'; ProfileFile = $claudeProfile; StateDirectory = $autoState; ClaudeArgs = @() }
            env = @{ OPS_BRAIN_TEST_CAPTURE = $capture; CLAUDE_CONFIG_DIR = $claudeBase }
        }
        Assert-True ($probe.exit -eq 0 -and $probe.error -eq '') "attended Auto launch failed: $($probe.error)"
        $liveCapture = [IO.File]::ReadAllText($capture)
        Assert-True ($liveCapture -like '*--dangerously-load-development-channels*') 'attended Auto did not open the Channel'
        Assert-True ($liveCapture -like '*"ops-brain-live"*') 'attended Auto overlay omitted the Channel definition'
        Assert-True ($probe.stderr -like "*ops-brain live: connecting as CC-CI (label claude-ci.scripts); adapter log: *claude-adapter.*.log*") "attended Auto banner missing or wrong: $($probe.stderr)"
        $liveConfigDirectory = (($liveCapture -split "`n---CONFIG-DIR---`n", 2)[1] -split "`n", 2)[0]
        Assert-True (-not (Test-Path -LiteralPath $liveConfigDirectory)) 'attended Auto left its overlay behind'

        # A subcommand on a console still passes through.
        $capture = Join-Path $testDirectory 'console-subcommand'
        $probe = Invoke-ConsoleProbe 'console-subcommand' @{ launcher = $claudeLauncher; parameters = @{ Mode = 'Auto'; ProfileFile = $claudeProfile; ClaudeArgs = @('mcp', 'list') }; env = @{ OPS_BRAIN_TEST_CAPTURE = $capture } }
        Assert-True ($probe.exit -eq 0 -and [IO.File]::ReadAllText("$capture.args") -eq "mcp`nlist") 'attended subcommand did not pass through'
        $capture = Join-Path $testDirectory 'console-codex-exec'
        $probe = Invoke-ConsoleProbe 'console-codex-exec' @{ launcher = $codexLauncher; parameters = @{ Mode = 'Auto'; ProfileFile = $codexProfile; CodexArgs = @('exec', 'do the thing') }; env = @{ OPS_BRAIN_TEST_CAPTURE = $capture } }
        Assert-True ($probe.exit -eq 0 -and [IO.File]::ReadAllText("$capture.args") -eq "exec`ndo the thing") 'attended codex exec did not pass through'
        Assert-True ($probe.stderr -notlike '*--remote*' -and $probe.stderr -notlike '*connecting as*') 'codex exec was routed through the App Server'

        # Attended, unconfigured: the operator is asked; "n" (and an empty
        # answer) exits 2 without launching, "y" launches ordinary and says so.
        $capture = Join-Path $testDirectory 'console-decline'
        $probe = Invoke-ConsoleProbe 'console-decline' @{ typed = "n`r"; launcher = $claudeLauncher; parameters = @{ Mode = 'Auto'; ProfileFile = $missingProfile; LiveUrl = 'wss://ops-brain.example/live'; ClaudeArgs = @() }; env = @{ OPS_BRAIN_TEST_CAPTURE = $capture } }
        Assert-True ($probe.exit -eq 2) "declined fallback exited $($probe.exit): $($probe.error)"
        Assert-True (-not (Test-Path -LiteralPath "$capture.args")) 'declined fallback launched Claude anyway'
        Assert-True ($probe.stderr -like '*ops-brain live: NOT available*AgentName is required*') "prompt reason missing: $($probe.stderr)"
        Assert-True ($probe.stderr -like '*declined ordinary fallback*') 'decline was not announced'
        $capture = Join-Path $testDirectory 'console-accept'
        $probe = Invoke-ConsoleProbe 'console-accept' @{ typed = "y`r"; launcher = $claudeLauncher; parameters = @{ Mode = 'Auto'; ProfileFile = $missingProfile; LiveUrl = 'wss://ops-brain.example/live'; ClaudeArgs = @('--resume') }; env = @{ OPS_BRAIN_TEST_CAPTURE = $capture } }
        Assert-True ($probe.exit -eq 0 -and [IO.File]::ReadAllText("$capture.args") -eq '--resume') "accepted fallback did not launch ordinary Claude: $($probe.error)"
        Assert-True ($probe.stderr -like '*ops-brain live: off (operator choice)*') 'accepted fallback was not announced'
        $capture = Join-Path $testDirectory 'console-codex-empty'
        $probe = Invoke-ConsoleProbe 'console-codex-empty' @{ typed = "`r"; launcher = $codexLauncher; parameters = @{ Mode = 'Auto'; ProfileFile = $missingProfile; LiveUrl = 'wss://ops-brain.example/live'; CodexArgs = @() }; env = @{ OPS_BRAIN_TEST_CAPTURE = $capture } }
        Assert-True ($probe.exit -eq 2 -and -not (Test-Path -LiteralPath "$capture.args")) 'Codex empty answer still launched'
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
