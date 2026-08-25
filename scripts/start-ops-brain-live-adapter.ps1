#requires -Version 7.4
# Credential-isolating child used by the Windows Claude and Codex launchers.

[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('claude', 'codex')]
    [string]$Client,
    [Parameter(Mandatory)][string]$Adapter,
    [Parameter(Mandatory)][string]$PowerShellPath,
    [Parameter(Mandatory)][uri]$LiveUrl,
    [Parameter(Mandatory)][string]$AgentCredentialFile,
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9._-]{1,80}$')]
    [string]$AgentName,
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9._-]{1,80}$')]
    [string]$Label,
    [uri]$AppServerUrl
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-NormalizedPath {
    param([Parameter(Mandatory)][string]$Path)
    [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($Path))
}

function Assert-LiveUrl {
    param([Parameter(Mandatory)][uri]$Url)
    if (-not $Url.IsAbsoluteUri) { throw 'LiveUrl must be an absolute wss://host/live URL' }
    $escapedPath = $Url.GetComponents([System.UriComponents]::Path, [System.UriFormat]::UriEscaped)
    if ($escapedPath -cne 'live' -or $Url.UserInfo -or
        $Url.OriginalString.Contains('?') -or $Url.OriginalString.Contains('#')) {
        throw 'LiveUrl must be exact wss://host/live without credentials, query, or fragment'
    }
    if ($Url.Scheme -ne 'wss' -and -not ($Url.Scheme -eq 'ws' -and $Url.IsLoopback)) {
        throw 'LiveUrl must use wss://, except ws:// loopback development endpoints'
    }
}

function Assert-AppServerUrl {
    param([Parameter(Mandatory)][uri]$Url)
    if (-not $Url.IsAbsoluteUri) { throw 'AppServerUrl must be an absolute WebSocket URL' }
    if ($Url.UserInfo -or $Url.Query -or $Url.Fragment) {
        throw 'AppServerUrl must not contain credentials, query, or fragment'
    }
    if ($Url.Scheme -notin @('ws', 'wss')) { throw 'AppServerUrl must use ws:// or wss://' }
    if ($Url.Scheme -eq 'ws' -and -not $Url.IsLoopback) {
        throw 'Plaintext AppServerUrl is allowed only on loopback'
    }
}

$Adapter = Get-NormalizedPath $Adapter
$PowerShellPath = Get-NormalizedPath $PowerShellPath
$AgentCredentialFile = Get-NormalizedPath $AgentCredentialFile
if (-not (Test-Path -LiteralPath $Adapter -PathType Leaf)) {
    throw "Adapter is missing: $Adapter"
}
if (-not (Test-Path -LiteralPath $PowerShellPath -PathType Leaf)) {
    throw "PowerShell executable is missing: $PowerShellPath"
}
Assert-LiveUrl $LiveUrl
if ($Client -eq 'codex' -and $null -eq $AppServerUrl) {
    throw 'AppServerUrl is required for the Codex adapter'
}
if ($null -ne $AppServerUrl) { Assert-AppServerUrl $AppServerUrl }

$nodeCommand = @(Get-Command node -CommandType Application -ErrorAction SilentlyContinue) | Select-Object -First 1
if ($null -eq $nodeCommand) { throw 'Required executable is missing: node' }
$tokenHelper = Join-Path $PSScriptRoot 'read-ops-brain-agent-token.ps1'
if (-not (Test-Path -LiteralPath $tokenHelper -PathType Leaf)) {
    throw "Agent token helper is missing: $tokenHelper"
}
$helperCommand = @(
    $PowerShellPath, '-NoLogo', '-NoProfile', '-NonInteractive', '-File', $tokenHelper,
    '-AgentCredentialFile', $AgentCredentialFile,
    '-AgentName', $AgentName
) | ConvertTo-Json -Compress
try {
    $env:OPS_BRAIN_LIVE_URL = $LiveUrl.AbsoluteUri
    $env:OPS_BRAIN_AGENT_TOKEN_HELPER_JSON = $helperCommand
    $env:OPS_BRAIN_EXPECTED_AGENT = $AgentName
    Remove-Item Env:OPS_BRAIN_AGENT_TOKEN -ErrorAction SilentlyContinue
    Remove-Item Env:OPS_BRAIN_AGENT_TOKEN_FILE -ErrorAction SilentlyContinue
    if ($Client -eq 'claude') {
        $env:OPS_BRAIN_LIVE_LABEL = $Label
    }
    else {
        $env:OPS_BRAIN_CODEX_LABEL = $Label
        $env:OPS_BRAIN_CODEX_APP_SERVER_URL = $AppServerUrl.AbsoluteUri
    }
    & $nodeCommand.Source $Adapter
    exit $LASTEXITCODE
}
finally {
    Remove-Item Env:OPS_BRAIN_AGENT_TOKEN -ErrorAction SilentlyContinue
    Remove-Item Env:OPS_BRAIN_AGENT_TOKEN_HELPER_JSON -ErrorAction SilentlyContinue
}
