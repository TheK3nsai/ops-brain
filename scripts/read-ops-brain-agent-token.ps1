#requires -Version 7.4
# Emit one DPAPI-protected agent token to the requesting adapter's private pipe.

[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$AgentCredentialFile,
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9._-]{1,80}$')]
    [string]$AgentName
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$path = [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($AgentCredentialFile))
if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
    throw "Agent credential is missing: $path"
}
$item = Get-Item -LiteralPath $path -Force
if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
    throw "Refusing credential reparse point: $path"
}
$credential = Import-Clixml -LiteralPath $path
if ($credential -isnot [System.Management.Automation.PSCredential]) {
    throw "Agent credential must be a DPAPI-protected PSCredential CliXml file: $path"
}
if (-not $credential.UserName.Equals($AgentName, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Credential identity does not match expected agent $AgentName"
}
$secret = $credential.GetNetworkCredential().Password
try {
    if ([string]::IsNullOrWhiteSpace($secret) -or $secret.Contains("`n") -or $secret.Contains("`r")) {
        throw 'Agent credential must contain one non-empty line'
    }
    [Console]::Out.Write($secret)
}
finally {
    $secret = $null
    $credential = $null
}
