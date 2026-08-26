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
    [string]$ProfileFile = $(if ($env:OPS_BRAIN_CLAUDE_PROFILE) { $env:OPS_BRAIN_CLAUDE_PROFILE } else { Join-Path $env:LOCALAPPDATA 'ops-brain\claude.json' }),
    [ValidatePattern('^[A-Za-z0-9._-]{1,80}$')]
    [string]$AgentName,
    [ValidatePattern('^[A-Za-z0-9._-]{1,80}$')]
    [string]$Label = $env:OPS_BRAIN_LIVE_LABEL,
    [string]$StateDirectory = $(if ($env:OPS_BRAIN_LIVE_STATE_DIR) { $env:OPS_BRAIN_LIVE_STATE_DIR } else { Join-Path $env:LOCALAPPDATA 'ops-brain-live' }),
    [Parameter(ValueFromRemainingArguments)]
    [string[]]$ClaudeArgs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-NormalizedPath {
    param([Parameter(Mandatory)][string]$Path)
    [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($Path))
}

function Assert-RealDirectory {
    param([Parameter(Mandatory)][string]$Path)
    $fullPath = Get-NormalizedPath $Path
    $rootPath = [IO.Path]::GetPathRoot($fullPath)
    if ($fullPath -eq $rootPath) { throw "Refusing unsafe state directory: $fullPath" }
    if (Test-Path -LiteralPath $fullPath) {
        $item = Get-Item -LiteralPath $fullPath -Force
        if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "State path must be a real directory, not a reparse point: $fullPath"
        }
    }
    else {
        [IO.Directory]::CreateDirectory($fullPath) | Out-Null
    }
    $fullPath
}

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

function Assert-AgentCredentialIdentity {
    param(
        [Parameter(Mandatory)][System.Management.Automation.ApplicationInfo]$PowerShell,
        [Parameter(Mandatory)][string]$TokenHelper,
        [Parameter(Mandatory)][string]$CredentialFile,
        [Parameter(Mandatory)][string]$ExpectedAgent
    )
    try {
        & $PowerShell.Source -NoLogo -NoProfile -NonInteractive -File $TokenHelper `
            -AgentCredentialFile $CredentialFile -AgentName $ExpectedAgent -ValidateOnly
        if ($LASTEXITCODE -ne 0) { throw "helper exited $LASTEXITCODE" }
    }
    catch {
        throw "Agent credential identity preflight failed for $ExpectedAgent"
    }
}

function Assert-LiveUrl {
    param([Parameter(Mandatory)][uri]$Url)
    if (-not $Url.IsAbsoluteUri) {
        throw 'LiveUrl must be an absolute wss://host/live URL'
    }
    $escapedPath = $Url.GetComponents([System.UriComponents]::Path, [System.UriFormat]::UriEscaped)
    if ($escapedPath -cne 'live' -or $Url.UserInfo -or
        $Url.OriginalString.Contains('?') -or $Url.OriginalString.Contains('#')) {
        throw 'LiveUrl must be exact wss://host/live without credentials, query, or fragment'
    }
    if ($Url.Scheme -ne 'wss' -and -not ($Url.Scheme -eq 'ws' -and $Url.IsLoopback)) {
        throw 'LiveUrl must use wss://, except ws:// loopback development endpoints'
    }
}

function Import-ClientProfile {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Client)
    $fullPath = [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($Path))
    if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) { return $null }
    $item = Get-Item -LiteralPath $fullPath -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Refusing profile reparse point: $fullPath" }
    $profile = Get-Content -LiteralPath $fullPath -Raw | ConvertFrom-Json
    if ($profile.schema -ne 1 -or $profile.client -ne $Client) { throw "Profile schema/client mismatch: $fullPath" }
    $profile
}

$ProfileFile = [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($ProfileFile))
$profile = $null
$profileError = $null
try {
    $profile = Import-ClientProfile $ProfileFile 'claude'
}
catch {
    if ($Mode -ne 'Status') { throw }
    $profileError = $_.Exception.Message
}
if ($null -ne $profile) {
    if ($null -eq $LiveUrl) { $LiveUrl = [uri]$profile.live_url }
    if (-not $AgentName) { $AgentName = [string]$profile.agent_name }
    if (-not $AgentCredentialFile) { $AgentCredentialFile = [string]$profile.credential_file }
    if (-not $Label) { $Label = [string]$profile.label }
}
if (-not $Label) { $Label = 'claude-code' }

$RepoDirectory = Split-Path -Parent $PSScriptRoot
$Adapter = Join-Path $RepoDirectory 'adapters\claude-channel\src\main.js'
$OverlayHelper = Join-Path $PSScriptRoot 'create-claude-channel-overlay.mjs'
$CredentialLauncher = Join-Path $PSScriptRoot 'start-ops-brain-live-adapter.ps1'
$TokenHelper = Join-Path $PSScriptRoot 'read-ops-brain-agent-token.ps1'
if ($AgentCredentialFile) {
    $AgentCredentialFile = [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($AgentCredentialFile))
}

$credentialStatus = if ($AgentCredentialFile -and (Test-Path -LiteralPath $AgentCredentialFile -PathType Leaf)) { 'present' } else { 'missing' }
if ($null -ne $LiveUrl) { Assert-LiveUrl $LiveUrl }
if ($Mode -eq 'Status') {
    "adapter: $Adapter"
    "profile: $ProfileFile $(if ($profileError) { '(not ready - {0})' -f $profileError } elseif ($null -ne $profile) { '(loaded)' } else { '(missing)' })"
    "live URL: $(if ($null -ne $LiveUrl) { $LiveUrl.AbsoluteUri } else { '<unset>' })"
    "label: $Label"
    "agent: $(if ($AgentName) { $AgentName } else { '<unset>' })"
    "credential: $(if ($AgentCredentialFile) { $AgentCredentialFile } else { '<unset>' }) - $credentialStatus"
    "claude: $(Get-ApplicationPath 'claude')"
    "node: $(Get-ApplicationPath 'node')"
    "channel: $(if (Test-Path -LiteralPath $OverlayHelper -PathType Leaf) { 'isolated per-launch user scope (ready)' } else { 'not ready (overlay helper missing)' })"
    "logs: $(Join-Path $StateDirectory 'claude-adapter.*.log')"
    exit 0
}

if ($null -eq $LiveUrl) { throw 'LiveUrl is required' }
if (-not $AgentName) { throw 'AgentName is required; select the exact Claude identity' }
if (-not $AgentCredentialFile) { throw 'AgentCredentialFile is required; select the exact Claude identity credential' }
if ($credentialStatus -ne 'present') { throw "Agent credential is missing: $AgentCredentialFile" }
if (-not (Test-Path -LiteralPath $Adapter -PathType Leaf)) { throw "Adapter is missing: $Adapter" }
if (-not (Test-Path -LiteralPath $OverlayHelper -PathType Leaf)) { throw "Claude Channel overlay helper is missing: $OverlayHelper" }
if (-not (Test-Path -LiteralPath $CredentialLauncher -PathType Leaf)) { throw "Credential launcher is missing: $CredentialLauncher" }
if (-not (Test-Path -LiteralPath $TokenHelper -PathType Leaf)) { throw "Agent token helper is missing: $TokenHelper" }
$pwshCommand = Get-RequiredApplication 'pwsh.exe'

# Reject a crossed identity before creating an overlay or spawning Claude. The
# helper's validation-only path checks DPAPI metadata without reading the bearer.
Assert-AgentCredentialIdentity $pwshCommand $TokenHelper $AgentCredentialFile $AgentName
$claudeCommand = Get-RequiredApplication 'claude'
$nodeCommand = Get-RequiredApplication 'node'

# Created only on a real launch, so Status stays free of side effects. A
# reparse point is refused for the same reason the Codex launcher refuses one:
# it would redirect adapter output to an attacker-chosen path written under the
# operator's own credentials.
$StateDirectory = Assert-RealDirectory $StateDirectory

$serverDefinition = @{
    command = $pwshCommand.Source
    args = @(
        '-NoLogo', '-NoProfile', '-NonInteractive', '-File', $CredentialLauncher,
        '-Client', 'claude', '-Adapter', $Adapter,
        '-PowerShellPath', $pwshCommand.Source,
        '-LiveUrl', $LiveUrl.AbsoluteUri,
        '-AgentCredentialFile', $AgentCredentialFile,
        '-AgentName', $AgentName,
        '-Label', $Label,
        '-StateDirectory', $StateDirectory
    )
} | ConvertTo-Json -Compress -Depth 6

function New-PrivateTemporaryDirectory {
    $directory = Join-Path ([IO.Path]::GetTempPath()) "ops-brain-claude-$([guid]::NewGuid().ToString('N'))"
    [IO.Directory]::CreateDirectory($directory) | Out-Null
    $item = Get-Item -LiteralPath $directory -Force
    if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Refusing unsafe Claude config overlay: $directory"
    }
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $security = [Security.AccessControl.DirectorySecurity]::new()
    $security.SetAccessRuleProtection($true, $false)
    $rule = [Security.AccessControl.FileSystemAccessRule]::new(
        $identity,
        [Security.AccessControl.FileSystemRights]::FullControl,
        [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
        [Security.AccessControl.PropagationFlags]::None,
        [Security.AccessControl.AccessControlType]::Allow
    )
    $security.AddAccessRule($rule)
    [IO.FileSystemAclExtensions]::SetAccessControl([IO.DirectoryInfo]$item, $security)
    $directory
}

if ($Mode -eq 'DryRun') {
    "would launch Claude with ops-brain online delivery through a private per-launch config overlay (credential path only; bearer redacted)"
    exit 0
}

$configDirectory = New-PrivateTemporaryDirectory
$previousConfigDirectory = $env:CLAUDE_CONFIG_DIR
try {
    & $nodeCommand.Source $OverlayHelper $configDirectory 'ops-brain-live' $serverDefinition
    if ($LASTEXITCODE -ne 0) { throw "Claude Channel overlay helper exited $LASTEXITCODE" }
    Remove-Item Env:OPS_BRAIN_AGENT_TOKEN -ErrorAction SilentlyContinue
    $env:CLAUDE_CONFIG_DIR = $configDirectory
    $userMcpConfig = if ($previousConfigDirectory) {
        Join-Path $previousConfigDirectory '.claude.json'
    }
    else {
        Join-Path $HOME '.claude.json'
    }
    $userMcpArguments = if (Test-Path -LiteralPath $userMcpConfig -PathType Leaf) {
        @('--mcp-config', $userMcpConfig)
    }
    else { @() }
    & $claudeCommand.Source @ClaudeArgs @userMcpArguments --dangerously-load-development-channels server:ops-brain-live
    exit $LASTEXITCODE
}
finally {
    if ($null -eq $previousConfigDirectory) {
        Remove-Item Env:CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue
    }
    else {
        $env:CLAUDE_CONFIG_DIR = $previousConfigDirectory
    }
    if (Test-Path -LiteralPath $configDirectory -PathType Container) {
        $resolved = [IO.Path]::GetFullPath($configDirectory)
        $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
        if ($resolved.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase) -and
            [IO.Path]::GetFileName($resolved).StartsWith('ops-brain-claude-', [StringComparison]::Ordinal)) {
            [IO.Directory]::Delete($resolved, $true)
        }
    }
}
