#requires -Version 7.2
# Credential-isolating child used by the Windows Claude and Codex launchers.

[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('claude', 'codex')]
    [string]$Client,
    [Parameter(Mandatory)][string]$Adapter,
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
    if ($Url.UserInfo -or $Url.Query -or $Url.Fragment) {
        throw 'AppServerUrl must not contain credentials, query, or fragment'
    }
    if ($Url.Scheme -notin @('ws', 'wss')) { throw 'AppServerUrl must use ws:// or wss://' }
    if ($Url.Scheme -eq 'ws' -and -not $Url.IsLoopback) {
        throw 'Plaintext AppServerUrl is allowed only on loopback'
    }
}

function Read-DpapiCredentialSecret {
    param([Parameter(Mandatory)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Agent credential is missing: $Path"
    }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "Refusing credential reparse point: $Path"
    }
    $credential = Import-Clixml -LiteralPath $Path
    if ($credential -isnot [System.Management.Automation.PSCredential]) {
        throw "Agent credential must be a DPAPI-protected PSCredential CliXml file: $Path"
    }
    if (-not $credential.UserName.Equals($AgentName, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Credential identity does not match expected agent $AgentName"
    }
    $secret = $credential.GetNetworkCredential().Password
    if ([string]::IsNullOrWhiteSpace($secret) -or $secret.Contains("`n") -or $secret.Contains("`r")) {
        throw 'Agent credential must contain one non-empty line'
    }
    $secret
}

$Adapter = Get-NormalizedPath $Adapter
$AgentCredentialFile = Get-NormalizedPath $AgentCredentialFile
if (-not (Test-Path -LiteralPath $Adapter -PathType Leaf)) {
    throw "Adapter is missing: $Adapter"
}
Assert-LiveUrl $LiveUrl
if ($Client -eq 'codex' -and $null -eq $AppServerUrl) {
    throw 'AppServerUrl is required for the Codex adapter'
}
if ($null -ne $AppServerUrl) { Assert-AppServerUrl $AppServerUrl }

$nodeCommand = Get-Command node -CommandType Application -ErrorAction Stop
$secret = Read-DpapiCredentialSecret $AgentCredentialFile
try {
    $env:OPS_BRAIN_LIVE_URL = $LiveUrl.AbsoluteUri
    $env:OPS_BRAIN_AGENT_TOKEN = $secret
    $env:OPS_BRAIN_EXPECTED_AGENT = $AgentName
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
    $secret = $null
    Remove-Item Env:OPS_BRAIN_AGENT_TOKEN -ErrorAction SilentlyContinue
}
