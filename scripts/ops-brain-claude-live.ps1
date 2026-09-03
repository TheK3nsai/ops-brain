#requires -Version 7.4
# Launch Claude Code with the local ops-brain Channel adapter on Windows.
#
# Modes:
#   Run     (default) live is required: any preflight failure fails closed
#   Auto    main-launcher mode for the OpsBrain-Shell.ps1 profile functions.
#           Attended console launches go live; redirected stdin/stdout,
#           -p/--print, subcommands, --version and --help pass straight
#           through to Claude Code untouched. A preflight failure asks the
#           operator on the console before continuing without live; it never
#           falls back silently.
#   Status  credential-safe report; DryRun validates preflight without launching
#   -NoLive (or a leading `--no-live` client argument, or OPS_BRAIN_LIVE=off)
#           launches ordinary Claude Code, announced once on stderr.

# PositionalBinding must stay off: with it on, the first trailing Claude argument
# binds to $LiveUrl by position instead of falling through to $ClaudeArgs, so
# `ops-brain-claude-live resume` dies in Assert-LiveUrl on a relative URI.
[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateSet('Run', 'Auto', 'Status', 'DryRun')]
    [string]$Mode = 'Run',
    [switch]$NoLive,
    [uri]$LiveUrl = $(if ($env:OPS_BRAIN_LIVE_URL) { $env:OPS_BRAIN_LIVE_URL } else { $null }),
    [string]$AgentCredentialFile = $env:OPS_BRAIN_AGENT_CREDENTIAL_FILE,
    [string]$ProfileFile = $(if ($env:OPS_BRAIN_CLAUDE_PROFILE) { $env:OPS_BRAIN_CLAUDE_PROFILE } else { Join-Path $env:LOCALAPPDATA 'ops-brain\claude.json' }),
    [ValidatePattern('^[A-Za-z0-9._-]{1,80}$')]
    [string]$AgentName,
    [ValidatePattern('^[A-Za-z0-9._-]{1,80}$')]
    [string]$Label = $env:OPS_BRAIN_LIVE_LABEL,
    [string]$StateDirectory = $(if ($env:OPS_BRAIN_LIVE_STATE_DIR) { $env:OPS_BRAIN_LIVE_STATE_DIR } else { Join-Path $env:LOCALAPPDATA 'ops-brain-live' }),
    # The profile function passes the client's arguments as one array through
    # this parameter: splatted or trailing tokens such as -p bind to launcher
    # parameters by prefix (-ProfileFile) and never reach Claude.
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

# With several attended sessions under one identity, the working directory is
# what tells them apart for a remote sender. The label stays self-reported,
# non-sensitive metadata; peer_id remains the only routing key.
function Get-LabelWithWorkingDirectory {
    param([Parameter(Mandatory)][string]$Base)
    $leaf = Split-Path -Leaf (Get-Location).Path
    $suffix = ($leaf -replace '[^A-Za-z0-9._-]', '-') -replace '^\.', ''
    if (-not $suffix) { return $Base }
    $combined = "$Base.$suffix"
    if ($combined.Length -gt 80) { $combined = $combined.Substring(0, 80) }
    $combined
}

# Ordinary Claude Code, exactly as if the launcher were not installed. Used
# for every path that must not open a live peer.
function Invoke-OrdinaryClient {
    param([string[]]$Arguments)
    $client = Get-RequiredApplication 'claude'
    & $client.Source @Arguments
    exit $LASTEXITCODE
}

# Auto only ever goes live for an attended foreground TUI. Everything a
# script, a wake shim, a redirected session, or a subcommand could be doing
# passes through untouched: a headless session cannot render a channel event,
# and a peer without a sink is the "healthy session, absent lane" defect.
function Test-AutoPassthrough {
    param([string[]]$Arguments)
    if (-not $script:attended) { return $true }
    foreach ($argument in $Arguments) {
        if ($argument -ceq '--') { break }
        if ($argument -cin @('-p', '--print', '-v', '--version', '-h', '--help')) { return $true }
    }
    if ($Arguments.Count -gt 0 -and -not $Arguments[0].StartsWith('-') -and $Arguments[0] -cin @(
            'mcp', 'auth', 'login', 'logout', 'doctor', 'update', 'install', 'plugin', 'plugins',
            'agents', 'config', 'setup-token', 'migrate-installer', 'remote-control', 'rc')) {
        return $true
    }
    $false
}

# A failed preflight in Auto asks the operator on the console; the answer is
# the affirmative choice the rollout contract requires before an ordinary
# session replaces a requested live one. Anything but an explicit yes exits
# nonzero. Outside Auto (the explicit command) the failure is final.
function Invoke-PreflightFailure {
    param([Parameter(Mandatory)][string]$Reason)
    if ($Mode -eq 'Auto' -and $script:attended) {
        [Console]::Error.WriteLine("ops-brain live: NOT available — $Reason")
        $answer = Read-Host 'Continue without live delivery? [y/N]'
        if ($answer -in @('y', 'yes')) {
            [Console]::Error.WriteLine('ops-brain live: off (operator choice); ordinary Claude Code session')
            Invoke-OrdinaryClient $ClaudeArgs
        }
        [Console]::Error.WriteLine('ops-brain live: declined ordinary fallback; not launching')
        exit 2
    }
    throw $Reason
}

if ($null -eq $ClaudeArgs) { $ClaudeArgs = @() }
# `claude --no-live` is the fleet-wide spelling. The profile function hands
# every client argument through -ClaudeArgs, so the opt-out arrives there.
if ($ClaudeArgs.Count -gt 0 -and $ClaudeArgs[0] -in @('--no-live', '-NoLive')) {
    $NoLive = $true
    $ClaudeArgs = @($ClaudeArgs | Select-Object -Skip 1)
}
if ($env:OPS_BRAIN_LIVE -and $env:OPS_BRAIN_LIVE.ToLowerInvariant() -in @('off', '0', 'false')) {
    $NoLive = $true
}
$script:attended = -not [Console]::IsInputRedirected -and -not [Console]::IsOutputRedirected

if ($Mode -in @('Run', 'Auto') -and $NoLive) {
    [Console]::Error.WriteLine('ops-brain live: off (requested); ordinary Claude Code session')
    Invoke-OrdinaryClient $ClaudeArgs
}
if ($Mode -eq 'Auto' -and (Test-AutoPassthrough $ClaudeArgs)) {
    Invoke-OrdinaryClient $ClaudeArgs
}

$ProfileFile = [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($ProfileFile))
$profile = $null
$profileError = $null
try {
    $profile = Import-ClientProfile $ProfileFile 'claude'
}
catch {
    $profileError = $_.Exception.Message
}
if ($null -ne $profile) {
    if ($null -eq $LiveUrl) { $LiveUrl = [uri]$profile.live_url }
    if (-not $AgentName) { $AgentName = [string]$profile.agent_name }
    if (-not $AgentCredentialFile) { $AgentCredentialFile = [string]$profile.credential_file }
    if (-not $Label) { $Label = [string]$profile.label }
}
if (-not $Label) { $Label = 'claude-code' }
if ($Mode -ne 'Status') { $Label = Get-LabelWithWorkingDirectory $Label }

$RepoDirectory = Split-Path -Parent $PSScriptRoot
$Adapter = Join-Path $RepoDirectory 'adapters\claude-channel\src\main.js'
$OverlayHelper = Join-Path $PSScriptRoot 'create-claude-channel-overlay.mjs'
$CredentialLauncher = Join-Path $PSScriptRoot 'start-ops-brain-live-adapter.ps1'
$TokenHelper = Join-Path $PSScriptRoot 'read-ops-brain-agent-token.ps1'
if ($AgentCredentialFile) {
    $AgentCredentialFile = [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($AgentCredentialFile))
}

$credentialStatus = if ($AgentCredentialFile -and (Test-Path -LiteralPath $AgentCredentialFile -PathType Leaf)) { 'present' } else { 'missing' }
if ($Mode -eq 'Status') {
    if ($null -ne $LiveUrl) { Assert-LiveUrl $LiveUrl }
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

$pwshCommand = $null
$claudeCommand = $null
$nodeCommand = $null
try {
    if ($profileError) { throw $profileError }
    if ($null -eq $LiveUrl) { throw 'LiveUrl is required' }
    Assert-LiveUrl $LiveUrl
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
}
catch {
    Invoke-PreflightFailure $_.Exception.Message
}

if ($Mode -eq 'DryRun') {
    "would launch Claude with ops-brain online delivery through a private per-launch config overlay (credential path only; bearer redacted)"
    "label: $Label"
    exit 0
}

# Created only on a real launch, so Status and DryRun stay free of side effects.
# A reparse point is refused for the same reason the Codex launcher refuses one:
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

function Remove-OverlayEntry {
    param([Parameter(Mandatory)][string]$Path)
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        if ($item.PSIsContainer) { [IO.Directory]::Delete($item.FullName, $false) }
        else { [IO.File]::Delete($item.FullName) }
    }
    elseif ($item.PSIsContainer) {
        foreach ($child in Get-ChildItem -LiteralPath $item.FullName -Force) {
            Remove-OverlayEntry $child.FullName
        }
        [IO.Directory]::Delete($item.FullName, $false)
    }
    else { [IO.File]::Delete($item.FullName) }
}

function Remove-PrivateTemporaryDirectory {
    param([Parameter(Mandatory)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { return }
    $resolved = [IO.Path]::GetFullPath($Path)
    $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    $item = Get-Item -LiteralPath $resolved -Force
    if (-not $resolved.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase) -or
        -not [IO.Path]::GetFileName($resolved).StartsWith('ops-brain-claude-', [StringComparison]::Ordinal) -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Refusing unsafe Claude config overlay cleanup: $resolved"
    }

    # Remove junctions as links instead of asking recursive deletion to traverse
    # them. Claude may also create ordinary directories in this writable overlay,
    # so recursively remove those without ever traversing a reparse point.
    foreach ($child in Get-ChildItem -LiteralPath $resolved -Force) {
        Remove-OverlayEntry $child.FullName
    }
    [IO.Directory]::Delete($resolved, $false)
}

# Each launch writes a new adapter log and nothing else prunes them; keep a
# month so a failed bind stays diagnosable without the directory growing for
# the life of the host.
function Remove-StaleAdapterLog {
    try {
        Get-ChildItem -LiteralPath $StateDirectory -File -Filter 'claude-adapter.*.log' -ErrorAction SilentlyContinue |
            Where-Object LastWriteTime -lt (Get-Date).AddDays(-30) |
            Remove-Item -Force -ErrorAction SilentlyContinue
    }
    catch { }
}

$configDirectory = New-PrivateTemporaryDirectory
$previousConfigDirectory = $env:CLAUDE_CONFIG_DIR
$claudeExitCode = 0
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
    Remove-StaleAdapterLog
    [Console]::Error.WriteLine("ops-brain live: connecting as $AgentName (label $Label); adapter log: $(Join-Path $StateDirectory 'claude-adapter.*.log')")
    & $claudeCommand.Source @ClaudeArgs @userMcpArguments --dangerously-load-development-channels server:ops-brain-live
    $claudeExitCode = $LASTEXITCODE
}
finally {
    if ($null -eq $previousConfigDirectory) {
        Remove-Item Env:CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue
    }
    else {
        $env:CLAUDE_CONFIG_DIR = $previousConfigDirectory
    }
    Remove-PrivateTemporaryDirectory $configDirectory
}
exit $claudeExitCode
