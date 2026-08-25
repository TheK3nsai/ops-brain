#requires -Version 7.4
# Launch one Codex TUI, App Server, and ops-brain adapter on Windows.

# PositionalBinding must stay off: with it on, the first trailing Codex argument
# binds to $LiveUrl by position instead of falling through to $CodexArgs, so
# `ops-brain-codex-live resume` dies in Assert-LiveUrl on a relative URI.
[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateSet('Run', 'Status', 'DryRun')]
    [string]$Mode = 'Run',
    [uri]$LiveUrl = $(if ($env:OPS_BRAIN_LIVE_URL) { $env:OPS_BRAIN_LIVE_URL } else { $null }),
    [string]$AgentCredentialFile = $env:OPS_BRAIN_AGENT_CREDENTIAL_FILE,
    [string]$ProfileFile = $(if ($env:OPS_BRAIN_CODEX_PROFILE) { $env:OPS_BRAIN_CODEX_PROFILE } else { Join-Path $env:LOCALAPPDATA 'ops-brain\codex.json' }),
    [ValidatePattern('^[A-Za-z0-9._-]{1,80}$')]
    [string]$AgentName,
    [ValidatePattern('^[A-Za-z0-9._-]{1,80}$')]
    [string]$Label = $env:OPS_BRAIN_CODEX_LABEL,
    [ValidateRange(1024, 65535)]
    [int]$AppServerPort = $(if ($env:OPS_BRAIN_CODEX_APP_SERVER_PORT) { [int]$env:OPS_BRAIN_CODEX_APP_SERVER_PORT } else { 4500 }),
    [string]$StateDirectory = $(if ($env:OPS_BRAIN_LIVE_STATE_DIR) { $env:OPS_BRAIN_LIVE_STATE_DIR } else { Join-Path $env:LOCALAPPDATA 'ops-brain-live' }),
    [Parameter(ValueFromRemainingArguments)]
    [string[]]$CodexArgs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-NormalizedPath {
    param([Parameter(Mandatory)][string]$Path)
    [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($Path))
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

function Stop-OwnedProcessTree {
    param([Diagnostics.Process]$Process)
    if ($null -eq $Process) { return }
    try {
        if (-not $Process.HasExited) {
            $Process.Kill($true)
            if (-not $Process.WaitForExit(3000)) { throw "Owned process $($Process.Id) did not exit" }
        }
    }
    catch [InvalidOperationException] { }
}

function Stop-AdapterProcess {
    param(
        [Diagnostics.Process]$Process,
        [Parameter(Mandatory)][string]$StopFile
    )
    if ($null -eq $Process) { return }
    try {
        if ($Process.HasExited) { return }
        [IO.File]::WriteAllText($StopFile, 'stop', [Text.UTF8Encoding]::new($false))
        if (-not $Process.WaitForExit(3000)) {
            Stop-OwnedProcessTree $Process
        }
    }
    catch [InvalidOperationException] { }
    catch { Stop-OwnedProcessTree $Process }
    finally {
        if (Test-Path -LiteralPath $StopFile -PathType Leaf) {
            Remove-Item -LiteralPath $StopFile -Force
        }
    }
}

function Test-AppServerReady {
    param([Parameter(Mandatory)][int]$Port)
    try {
        $response = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/readyz" -TimeoutSec 1 -SkipHttpErrorCheck
        return [int]$response.StatusCode -eq 200
    }
    catch { return $false }
}

function ConvertTo-ProcessArgument {
    param([Parameter(Mandatory)][string]$Value)
    if ($Value.Contains('"')) { throw 'Process arguments may not contain a double quote' }
    if ($Value -match '\s') { return '"' + $Value + '"' }
    $Value
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
    $profile = Import-ClientProfile $ProfileFile 'codex'
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
    if (-not $PSBoundParameters.ContainsKey('AppServerPort') -and -not $env:OPS_BRAIN_CODEX_APP_SERVER_PORT) {
        $AppServerPort = [int]$profile.app_server_port
    }
}
if (-not $Label) { $Label = 'codex-live' }

$RepoDirectory = Split-Path -Parent $PSScriptRoot
$Adapter = Join-Path $RepoDirectory 'adapters\codex-app-server\src\index.mjs'
$CredentialLauncher = Join-Path $PSScriptRoot 'start-ops-brain-live-adapter.ps1'
$TokenHelper = Join-Path $PSScriptRoot 'read-ops-brain-agent-token.ps1'
$AppServerUrl = [uri]"ws://127.0.0.1:$AppServerPort"
# codex app-server --listen rejects any path component -- `--help` documents the
# supported form as bare ws://IP:PORT. [uri].AbsoluteUri normalizes the empty path to
# "/", producing ws://127.0.0.1:4500/, which the parser refuses outright. Keep the [uri]
# above for validation and display; pass this bare string to codex.exe itself.
$AppServerEndpoint = "ws://127.0.0.1:$AppServerPort"
$StateDirectory = Get-NormalizedPath $StateDirectory
if ($AgentCredentialFile) {
    $AgentCredentialFile = Get-NormalizedPath $AgentCredentialFile
}
$credentialStatus = if ($AgentCredentialFile -and (Test-Path -LiteralPath $AgentCredentialFile -PathType Leaf)) { 'present' } else { 'missing' }
if ($null -ne $LiveUrl) { Assert-LiveUrl $LiveUrl }

if ($Mode -eq 'Status') {
    "adapter: $Adapter"
    "profile: $ProfileFile $(if ($profileError) { '(not ready - {0})' -f $profileError } elseif ($null -ne $profile) { '(loaded)' } else { '(missing)' })"
    "live URL: $(if ($null -ne $LiveUrl) { $LiveUrl.AbsoluteUri } else { '<unset>' })"
    "label: $Label"
    "agent: $(if ($AgentName) { $AgentName } else { '<unset>' })"
    "App Server: $AppServerEndpoint"
    "credential: $(if ($AgentCredentialFile) { $AgentCredentialFile } else { '<unset>' }) - $credentialStatus"
    "state: $StateDirectory"
    "codex: $(Get-ApplicationPath 'codex.exe')"
    "node: $(Get-ApplicationPath 'node')"
    exit 0
}

if ($null -eq $LiveUrl) { throw 'LiveUrl is required' }
if (-not $AgentName) { throw 'AgentName is required; select the exact Codex identity' }
if (-not $AgentCredentialFile) { throw 'AgentCredentialFile is required; select the exact Codex identity credential' }
if ($credentialStatus -ne 'present') { throw "Agent credential is missing: $AgentCredentialFile" }
if (-not (Test-Path -LiteralPath $Adapter -PathType Leaf)) { throw "Adapter is missing: $Adapter" }
if (-not (Test-Path -LiteralPath $CredentialLauncher -PathType Leaf)) { throw "Credential launcher is missing: $CredentialLauncher" }
if (-not (Test-Path -LiteralPath $TokenHelper -PathType Leaf)) { throw "Agent token helper is missing: $TokenHelper" }
$pwshCommand = Get-RequiredApplication 'pwsh.exe'

# Reject a crossed identity before starting the owned App Server or Codex TUI.
# Validation-only checks DPAPI metadata without reading the bearer.
Assert-AgentCredentialIdentity $pwshCommand $TokenHelper $AgentCredentialFile $AgentName
$codexCommand = Get-RequiredApplication 'codex.exe'
[void](Get-RequiredApplication 'node')

if ($Mode -eq 'DryRun') {
    "would launch App Server at $AppServerEndpoint"
    'would launch the Codex adapter with a DPAPI credential (bearer redacted)'
    'would launch one Codex TUI through that App Server'
    exit 0
}

Remove-Item Env:OPS_BRAIN_AGENT_TOKEN -ErrorAction SilentlyContinue
$StateDirectory = Assert-RealDirectory $StateDirectory
$runId = [guid]::NewGuid().ToString('N')
$appOut = Join-Path $StateDirectory "app-server.$runId.stdout.log"
$appErr = Join-Path $StateDirectory "app-server.$runId.stderr.log"
$adapterOut = Join-Path $StateDirectory "codex-adapter.$runId.stdout.log"
$adapterErr = Join-Path $StateDirectory "codex-adapter.$runId.stderr.log"
$adapterStopFile = Join-Path $StateDirectory "codex-adapter.$runId.stop"
$appProcess = $null
$adapterProcess = $null

try {
    if (Test-AppServerReady $AppServerPort) {
        throw "Port $AppServerPort already hosts an App Server; select another port"
    }

    # Hidden, NOT -NoNewWindow. Redirecting output sets UseShellExecute=false, so this
    # requires PowerShell 7.4/.NET 8; earlier runtimes silently ignored WindowStyle in
    # that mode. Without either, Start-Process gives this child its own
    # console; because stdout and stderr are redirected to files it renders as an empty
    # terminal beside the TUI, looks like stray junk, and closing it kills the live lane
    # with no log entry at all (observed during the 2026-08-17 acceptance gate).
    #
    # -NoNewWindow fixes the closable window but breaks the gate a different way: the
    # child then shares the launcher's console and holds its input handle, so the Codex
    # TUI stops accepting keystrokes. Hidden keeps the child on its own console, off
    # screen and away from the TUI's stdin.
    $appProcess = Start-Process -FilePath $codexCommand.Source -ArgumentList @('app-server', '--listen', $AppServerEndpoint) -RedirectStandardOutput $appOut -RedirectStandardError $appErr -WindowStyle Hidden -PassThru
    $ready = $false
    foreach ($attempt in 1..50) {
        if ($appProcess.HasExited) { break }
        if (Test-AppServerReady $AppServerPort) { $ready = $true; break }
        Start-Sleep -Milliseconds 100
    }
    if (-not $ready) { throw "App Server failed to become ready; see $appErr" }

    $adapterArguments = @(
        '-NoLogo', '-NoProfile', '-NonInteractive', '-File', $CredentialLauncher,
        '-Client', 'codex', '-Adapter', $Adapter,
        '-PowerShellPath', $pwshCommand.Source,
        '-LiveUrl', $LiveUrl.AbsoluteUri,
        '-AgentCredentialFile', $AgentCredentialFile,
        '-AgentName', $AgentName,
        '-Label', $Label,
        '-AppServerUrl', $AppServerUrl.AbsoluteUri
    )
    # Hidden for the same reasons as the App Server above. This is the process whose
    # death removes the live peer, so it must be neither closable nor able to contend
    # for the TUI's console input.
    $previousStopFile = $env:OPS_BRAIN_LIVE_STOP_FILE
    try {
        $env:OPS_BRAIN_LIVE_STOP_FILE = $adapterStopFile
        $adapterProcess = Start-Process -FilePath $pwshCommand.Source -ArgumentList @($adapterArguments | ForEach-Object { ConvertTo-ProcessArgument $_ }) -RedirectStandardOutput $adapterOut -RedirectStandardError $adapterErr -WindowStyle Hidden -PassThru
    }
    finally {
        if ($null -eq $previousStopFile) {
            Remove-Item Env:OPS_BRAIN_LIVE_STOP_FILE -ErrorAction SilentlyContinue
        }
        else { $env:OPS_BRAIN_LIVE_STOP_FILE = $previousStopFile }
    }

    & $codexCommand.Source --remote $AppServerEndpoint @CodexArgs
    exit $LASTEXITCODE
}
finally {
    # Let the adapter close its WebSocket before the owned process tree is reaped.
    # Abrupt termination can leave the remote peer visible until proxy TCP expiry.
    Stop-AdapterProcess $adapterProcess $adapterStopFile
    Stop-OwnedProcessTree $appProcess
}
