#requires -Version 7.2
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
    & "$PSScriptRoot\Install-OpsBrainLive.ps1" -SkipDependencies -BinDirectory $binDirectory
    & "$PSScriptRoot\Install-OpsBrainLive.ps1" -Mode Status -BinDirectory $binDirectory
    & "$PSScriptRoot\ops-brain-claude-live.ps1" -Mode Status
    & "$PSScriptRoot\ops-brain-codex-live.ps1" -Mode Status

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
    $fakeClaude = Join-Path $fakeBin 'claude.exe'
    $source = @'
using System;
using System.IO;
public static class FakeClaude {
    public static int Main(string[] args) {
        if (!String.IsNullOrEmpty(Environment.GetEnvironmentVariable("OPS_BRAIN_AGENT_TOKEN"))) return 3;
        var capture = Environment.GetEnvironmentVariable("OPS_BRAIN_TEST_CAPTURE");
        var marker = Array.IndexOf(args, "--mcp-config");
        if (String.IsNullOrEmpty(capture) || marker < 0 || marker + 1 >= args.Length) return 2;
        var config = File.ReadAllText(args[marker + 1]);
        File.WriteAllText(capture, config + "\n---ARGS---\n" + String.Join("\n", args));
        return 0;
    }
}
'@
    Add-Type -TypeDefinition $source -OutputAssembly $fakeClaude -OutputType ConsoleApplication

    $credential = Join-Path $testDirectory 'credential.cred.xml'
    [IO.File]::WriteAllText($credential, 'credential fixture is not read by the parent launcher')
    $originalPath = $env:PATH
    $env:PATH = "$fakeBin;$originalPath"
    $env:OPS_BRAIN_TEST_CAPTURE = $captureFile
    $env:OPS_BRAIN_AGENT_TOKEN = 'fixture-must-not-reach-claude'
    try {
        $pwsh = (@(Get-Command pwsh.exe -CommandType Application -ErrorAction Stop) | Select-Object -First 1).Source
        $startInfo = [Diagnostics.ProcessStartInfo]::new($pwsh)
        $startInfo.UseShellExecute = $false
        foreach ($argument in @(
            '-NoLogo', '-NoProfile', '-File', "$PSScriptRoot\ops-brain-claude-live.ps1",
            '-LiveUrl', 'wss://ops-brain.example/live',
            '-AgentCredentialFile', $credential,
            '-AgentName', 'CC-CI',
            '-Label', 'claude-ci'
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
    $parts = $capture -split "`n---ARGS---`n", 2
    Assert-True ($parts.Count -eq 2) 'fake Claude did not receive config and arguments'
    $config = $parts[0] | ConvertFrom-Json
    $server = $config.mcpServers.'ops-brain-live'
    Assert-True ($server.command -eq 'pwsh.exe') 'generated Claude MCP command is wrong'
    Assert-True ($server.args -contains '-AgentName') 'generated Claude MCP config omitted AgentName'
    Assert-True ($server.args -contains 'CC-CI') 'generated Claude MCP config omitted the expected identity'
    Assert-True ($capture -notlike '*credential fixture is not read*') 'credential contents leaked into Claude capture'
    $arguments = @($parts[1] -split "`n")
    $configIndex = [Array]::IndexOf($arguments, '--mcp-config')
    Assert-True ($configIndex -ge 0 -and $configIndex + 1 -lt $arguments.Count) 'Claude did not receive one MCP config path argument'
    Assert-True (-not (Test-Path -LiteralPath $arguments[$configIndex + 1])) 'temporary Claude MCP config was not removed'
    Assert-True ($arguments -contains '--dangerously-load-development-channels') 'Claude Channel opt-in flag is missing'

    'Windows live launcher tests passed'
}
finally {
    $resolved = [IO.Path]::GetFullPath($testDirectory)
    if ($resolved.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase) -and
        [IO.Path]::GetFileName($resolved).StartsWith('ops-brain-live-windows-test-', [StringComparison]::Ordinal)) {
        [IO.Directory]::Delete($resolved, $true)
    }
}
