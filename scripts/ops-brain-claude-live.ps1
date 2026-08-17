#requires -Version 7.4
# Launch Claude Code with the local ops-brain Channel adapter on Windows.

# PositionalBinding must stay off: with it on, the first trailing Claude argument
# binds to $LiveUrl by position instead of falling through to $ClaudeArgs, so
# `ops-brain-claude-live resume` dies in Assert-LiveUrl on a relative URI.
[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateSet('Run', 'Status', 'DryRun')]
    [string]$Mode = 'Run',
    [uri]$LiveUrl = $(if ($env:OPS_BRAIN_LIVE_URL) { $env:OPS_BRAIN_LIVE_URL } else { $null }),
    [string]$AgentCredentialFile = $env:OPS_BRAIN_AGENT_CREDENTIAL_FILE,
    [ValidatePattern('^[A-Za-z0-9._-]{1,80}$')]
    [string]$AgentName,
    [ValidatePattern('^[A-Za-z0-9._-]{1,80}$')]
    [string]$Label = $(if ($env:OPS_BRAIN_LIVE_LABEL) { $env:OPS_BRAIN_LIVE_LABEL } else { 'claude-code' }),
    [Parameter(ValueFromRemainingArguments)]
    [string[]]$ClaudeArgs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-ApplicationPath {
    param([Parameter(Mandatory)][string]$Name)
    $command = @(Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue) | Select-Object -First 1
    if ($null -eq $command) { return '<missing>' }
    $command.Source
}

function Get-RequiredApplication {
    param([Parameter(Mandatory)][string]$Name)
    $command = @(Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue) | Select-Object -First 1
    if ($null -eq $command) { throw "Required executable is missing: $Name" }
    $command
}

function Assert-LiveUrl {
    param([Parameter(Mandatory)][uri]$Url)
    $escapedPath = $Url.GetComponents([System.UriComponents]::Path, [System.UriFormat]::UriEscaped)
    if ($escapedPath -cne 'live' -or $Url.UserInfo -or
        $Url.OriginalString.Contains('?') -or $Url.OriginalString.Contains('#')) {
        throw 'LiveUrl must be exact wss://host/live without credentials, query, or fragment'
    }
    if ($Url.Scheme -ne 'wss' -and -not ($Url.Scheme -eq 'ws' -and $Url.IsLoopback)) {
        throw 'LiveUrl must use wss://, except ws:// loopback development endpoints'
    }
}

$RepoDirectory = Split-Path -Parent $PSScriptRoot
$Adapter = Join-Path $RepoDirectory 'adapters\claude-channel\src\main.js'
$CredentialLauncher = Join-Path $PSScriptRoot 'start-ops-brain-live-adapter.ps1'
if ($AgentCredentialFile) {
    $AgentCredentialFile = [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($AgentCredentialFile))
}

$credentialStatus = if ($AgentCredentialFile -and (Test-Path -LiteralPath $AgentCredentialFile -PathType Leaf)) { 'present' } else { 'missing' }
if ($null -ne $LiveUrl) { Assert-LiveUrl $LiveUrl }
if ($Mode -eq 'Status') {
    "adapter: $Adapter"
    "live URL: $(if ($null -ne $LiveUrl) { $LiveUrl.AbsoluteUri } else { '<unset>' })"
    "label: $Label"
    "agent: $(if ($AgentName) { $AgentName } else { '<unset>' })"
    "credential: $(if ($AgentCredentialFile) { $AgentCredentialFile } else { '<unset>' }) - $credentialStatus"
    "claude: $(Get-ApplicationPath 'claude')"
    "node: $(Get-ApplicationPath 'node')"
    exit 0
}

if ($null -eq $LiveUrl) { throw 'LiveUrl is required' }
if (-not $AgentName) { throw 'AgentName is required; select the exact Claude identity' }
if (-not $AgentCredentialFile) { throw 'AgentCredentialFile is required; select the exact Claude identity credential' }
if ($credentialStatus -ne 'present') { throw "Agent credential is missing: $AgentCredentialFile" }
if (-not (Test-Path -LiteralPath $Adapter -PathType Leaf)) { throw "Adapter is missing: $Adapter" }
if (-not (Test-Path -LiteralPath $CredentialLauncher -PathType Leaf)) { throw "Credential launcher is missing: $CredentialLauncher" }
$claudeCommand = Get-RequiredApplication 'claude'
[void](Get-RequiredApplication 'node')
[void](Get-RequiredApplication 'pwsh.exe')

$mcpConfig = @{
    mcpServers = @{
        'ops-brain-live' = @{
            command = 'pwsh.exe'
            args = @(
                '-NoLogo', '-NoProfile', '-File', $CredentialLauncher,
                '-Client', 'claude', '-Adapter', $Adapter,
                '-LiveUrl', $LiveUrl.AbsoluteUri,
                '-AgentCredentialFile', $AgentCredentialFile,
                '-AgentName', $AgentName,
                '-Label', $Label
            )
        }
    }
} | ConvertTo-Json -Compress -Depth 6

if ($Mode -eq 'DryRun') {
    "would launch Claude with the ops-brain development Channel (credential path only; bearer redacted)"
    exit 0
}

$configFile = [IO.Path]::GetTempFileName()
try {
    $configItem = Get-Item -LiteralPath $configFile -Force
    if ($configItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "Refusing temporary MCP config reparse point: $configFile"
    }
    [IO.File]::WriteAllText($configFile, $mcpConfig, [Text.UTF8Encoding]::new($false))
    Remove-Item Env:OPS_BRAIN_AGENT_TOKEN -ErrorAction SilentlyContinue
    & $claudeCommand.Source @ClaudeArgs --mcp-config $configFile --dangerously-load-development-channels server:ops-brain-live
    exit $LASTEXITCODE
}
finally {
    if (Test-Path -LiteralPath $configFile -PathType Leaf) {
        Remove-Item -LiteralPath $configFile -Force
    }
}
