#requires -Version 7.4
# Launch one Codex TUI, App Server, and ops-brain adapter on Windows.
#
# Modes:
#   Run     (default) live is required: any preflight failure fails closed
#   Auto    main-launcher mode for the OpsBrain-Shell.ps1 profile functions.
#           Attended console launches go live; redirected stdin/stdout,
#           `codex exec` and every other subcommand, --version and --help pass
#           straight through to Codex untouched. When the profile's App Server
#           port is already in use by an earlier live session, a free loopback
#           port is chosen so a second attended session gets its own App Server
#           and adapter. A preflight failure asks the operator on the console
#           before continuing without live; it never falls back silently.
#   Status  credential-safe report; DryRun validates preflight without launching
#   -NoLive (or a leading `--no-live` client argument, or OPS_BRAIN_LIVE=off)
#           launches ordinary Codex, announced once on stderr.

# PositionalBinding must stay off: with it on, the first trailing Codex argument
# binds to $LiveUrl by position instead of falling through to $CodexArgs, so
# `ops-brain-codex-live resume` dies in Assert-LiveUrl on a relative URI.
[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateSet('Run', 'Auto', 'Status', 'DryRun')]
    [string]$Mode = 'Run',
    [switch]$NoLive,
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
    # The profile function passes the client's arguments as one array through
    # this parameter; see the Claude launcher for why trailing tokens cannot.
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

function Get-NativeCodexPath {
    $command = @(Get-Command 'codex.exe' -CommandType Application -ErrorAction SilentlyContinue) | Select-Object -First 1
    if ($null -ne $command) { return $command.Source }

    $npmRoots = [Collections.Generic.List[string]]::new()
    $shim = @(Get-Command 'codex.cmd' -CommandType Application -ErrorAction SilentlyContinue) | Select-Object -First 1
    if ($null -ne $shim) { [void]$npmRoots.Add((Split-Path -Parent $shim.Source)) }
    if ($env:APPDATA) { [void]$npmRoots.Add((Join-Path $env:APPDATA 'npm')) }
    $platforms = @(
        @{ Package = 'codex-win32-x64'; Triple = 'x86_64-pc-windows-msvc' },
        @{ Package = 'codex-win32-arm64'; Triple = 'aarch64-pc-windows-msvc' }
    )
    foreach ($root in @($npmRoots | Select-Object -Unique)) {
        foreach ($platform in $platforms) {
            foreach ($relative in @(
                "node_modules\@openai\codex\node_modules\@openai\$($platform.Package)\vendor\$($platform.Triple)\bin\codex.exe",
                "node_modules\@openai\$($platform.Package)\vendor\$($platform.Triple)\bin\codex.exe"
            )) {
                $candidate = Join-Path $root $relative
                if (Test-Path -LiteralPath $candidate -PathType Leaf) {
                    return [IO.Path]::GetFullPath($candidate)
                }
            }
        }
    }
    '<missing>'
}

function Get-RequiredNativeCodexPath {
    $path = Get-NativeCodexPath
    if ($path -eq '<missing>') { throw 'Required executable is missing: native codex.exe' }
    $path
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

function Get-FreeLoopbackPort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    try { ([Net.IPEndPoint]$listener.LocalEndpoint).Port }
    finally { $listener.Stop() }
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

# See the Claude launcher: the working directory tells sibling sessions apart.
function Get-LabelWithWorkingDirectory {
    param([Parameter(Mandatory)][string]$Base)
    $leaf = Split-Path -Leaf (Get-Location).Path
    $suffix = ($leaf -replace '[^A-Za-z0-9._-]', '-') -replace '^\.', ''
    if (-not $suffix) { return $Base }
    $combined = "$Base.$suffix"
    if ($combined.Length -gt 80) { $combined = $combined.Substring(0, 80) }
    $combined
}

# Ordinary Codex, exactly as plain `codex` resolves on this host (the npm shim
# or a native binary on PATH). Used for every path that must not open a peer.
function Invoke-OrdinaryClient {
    param([string[]]$Arguments)
    $client = Get-RequiredApplication 'codex'
    & $client.Source @Arguments
    exit $LASTEXITCODE
}

# Auto only goes live for an attended foreground TUI. A subcommand (exec, mcp,
# login, app-server, ...) is never a TUI, and a redirected stdin makes
# `codex --remote` exit before the adapter could ever bind a thread.
function Test-AutoPassthrough {
    param([string[]]$Arguments)
    if (-not $script:attended) { return $true }
    foreach ($argument in $Arguments) {
        if ($argument -ceq '--') { break }
        if ($argument -cin @('-V', '--version', '-h', '--help')) { return $true }
    }
    if ($Arguments.Count -gt 0 -and -not $Arguments[0].StartsWith('-')) { return $true }
    $false
}

# See the Claude launcher: in Auto on a console a failed preflight asks before
# an ordinary session replaces the requested live one; the explicit command
# fails closed.
function Invoke-PreflightFailure {
    param([Parameter(Mandatory)][string]$Reason)
    if ($Mode -eq 'Auto' -and $script:attended) {
        [Console]::Error.WriteLine("ops-brain live: NOT available — $Reason")
        $answer = Read-Host 'Continue without live delivery? [y/N]'
        if ($answer -in @('y', 'yes')) {
            [Console]::Error.WriteLine('ops-brain live: off (operator choice); ordinary Codex session')
            Invoke-OrdinaryClient $CodexArgs
        }
        [Console]::Error.WriteLine('ops-brain live: declined ordinary fallback; not launching')
        exit 2
    }
    throw $Reason
}

if ($null -eq $CodexArgs) { $CodexArgs = @() }
if ($CodexArgs.Count -gt 0 -and $CodexArgs[0] -in @('--no-live', '-NoLive')) {
    $NoLive = $true
    $CodexArgs = @($CodexArgs | Select-Object -Skip 1)
}
if ($env:OPS_BRAIN_LIVE -and $env:OPS_BRAIN_LIVE.ToLowerInvariant() -in @('off', '0', 'false')) {
    $NoLive = $true
}
$script:attended = -not [Console]::IsInputRedirected -and -not [Console]::IsOutputRedirected

if ($Mode -in @('Run', 'Auto') -and $NoLive) {
    [Console]::Error.WriteLine('ops-brain live: off (requested); ordinary Codex session')
    Invoke-OrdinaryClient $CodexArgs
}
if ($Mode -eq 'Auto' -and (Test-AutoPassthrough $CodexArgs)) {
    Invoke-OrdinaryClient $CodexArgs
}

$ProfileFile = [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($ProfileFile))
$profile = $null
$profileError = $null
try {
    $profile = Import-ClientProfile $ProfileFile 'codex'
}
catch {
    $profileError = $_.Exception.Message
}
$portExplicit = $PSBoundParameters.ContainsKey('AppServerPort') -or [bool]$env:OPS_BRAIN_CODEX_APP_SERVER_PORT
if ($null -ne $profile) {
    if ($null -eq $LiveUrl) { $LiveUrl = [uri]$profile.live_url }
    if (-not $AgentName) { $AgentName = [string]$profile.agent_name }
    if (-not $AgentCredentialFile) { $AgentCredentialFile = [string]$profile.credential_file }
    if (-not $Label) { $Label = [string]$profile.label }
    if (-not $portExplicit) {
        $AppServerPort = [int]$profile.app_server_port
    }
}
if (-not $Label) { $Label = 'codex-live' }
if ($Mode -ne 'Status') { $Label = Get-LabelWithWorkingDirectory $Label }

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

if ($Mode -eq 'Status') {
    if ($null -ne $LiveUrl) { Assert-LiveUrl $LiveUrl }
    "adapter: $Adapter"
    "profile: $ProfileFile $(if ($profileError) { '(not ready - {0})' -f $profileError } elseif ($null -ne $profile) { '(loaded)' } else { '(missing)' })"
    "live URL: $(if ($null -ne $LiveUrl) { $LiveUrl.AbsoluteUri } else { '<unset>' })"
    "label: $Label"
    "agent: $(if ($AgentName) { $AgentName } else { '<unset>' })"
    "App Server: $AppServerEndpoint"
    "credential: $(if ($AgentCredentialFile) { $AgentCredentialFile } else { '<unset>' }) - $credentialStatus"
    "state: $StateDirectory"
    "codex: $(Get-NativeCodexPath)"
    "node: $(Get-ApplicationPath 'node')"
    exit 0
}

$pwshCommand = $null
$codexCommand = $null
try {
    if ($profileError) { throw $profileError }
    if ($null -eq $LiveUrl) { throw 'LiveUrl is required' }
    Assert-LiveUrl $LiveUrl
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
    $codexCommand = Get-RequiredNativeCodexPath
    [void](Get-RequiredApplication 'node')
}
catch {
    Invoke-PreflightFailure $_.Exception.Message
}

if ($Mode -eq 'DryRun') {
    "would launch App Server at $AppServerEndpoint"
    'would launch the Codex adapter with a DPAPI credential (bearer redacted)'
    'would launch one Codex TUI through that App Server'
    "label: $Label"
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
        if ($Mode -eq 'Auto' -and -not $portExplicit) {
            # An earlier attended session owns the profile port. Each live TUI
            # needs its own App Server so the adapter's exactly-one-thread
            # invariant holds per session; pick a free loopback port instead.
            $chosen = Get-FreeLoopbackPort
            [Console]::Error.WriteLine("ops-brain live: port $AppServerPort already hosts an App Server; using $chosen for this session")
            $AppServerPort = $chosen
            $AppServerUrl = [uri]"ws://127.0.0.1:$AppServerPort"
            $AppServerEndpoint = "ws://127.0.0.1:$AppServerPort"
        }
        else {
            throw "Port $AppServerPort already hosts an App Server; select another port"
        }
    }

    # Each launch writes new helper logs and nothing else prunes them; keep a month.
    try {
        Get-ChildItem -LiteralPath $StateDirectory -File -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -like 'codex-adapter.*.log' -or $_.Name -like 'app-server.*.log' } |
            Where-Object LastWriteTime -lt (Get-Date).AddDays(-30) |
            Remove-Item -Force -ErrorAction SilentlyContinue
    }
    catch { }

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
    $appProcess = Start-Process -FilePath $codexCommand -ArgumentList @('app-server', '--listen', $AppServerEndpoint) -RedirectStandardOutput $appOut -RedirectStandardError $appErr -WindowStyle Hidden -PassThru
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

    # The adapter's structured log lines go to its stderr file.
    [Console]::Error.WriteLine("ops-brain live: connecting as $AgentName (label $Label); adapter log: $adapterErr")
    & $codexCommand --remote $AppServerEndpoint @CodexArgs
    exit $LASTEXITCODE
}
finally {
    # Let the adapter close its WebSocket before the owned process tree is reaped.
    # Abrupt termination can leave the remote peer visible until proxy TCP expiry.
    Stop-AdapterProcess $adapterProcess $adapterStopFile
    Stop-OwnedProcessTree $appProcess
}
