#requires -Version 7.4
# Install ops-brain foreground client launch shims for one Windows user.

[CmdletBinding()]
param(
    [ValidateSet('Install', 'Status')]
    [string]$Mode = 'Install',
    [string]$BinDirectory = $(Join-Path $env:LOCALAPPDATA 'Programs\ops-brain'),
    [switch]$SkipDependencies
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$RepoDirectory = Split-Path -Parent $PSScriptRoot
$BinDirectory = [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($BinDirectory))
$launchers = @('ops-brain-client', 'ops-brain-claude', 'ops-brain-codex', 'ops-brain-claude-live', 'ops-brain-codex-live')
$targets = @{
    'ops-brain-client' = @{ Kind = 'node'; Path = (Join-Path $PSScriptRoot 'ops-brain-client') }
    'ops-brain-claude' = @{ Kind = 'pwsh'; Path = (Join-Path $PSScriptRoot 'ops-brain-claude-live.ps1') }
    'ops-brain-codex' = @{ Kind = 'pwsh'; Path = (Join-Path $PSScriptRoot 'ops-brain-codex-live.ps1') }
    'ops-brain-claude-live' = @{ Kind = 'pwsh'; Path = (Join-Path $PSScriptRoot 'ops-brain-claude-live.ps1') }
    'ops-brain-codex-live' = @{ Kind = 'pwsh'; Path = (Join-Path $PSScriptRoot 'ops-brain-codex-live.ps1') }
}

function Assert-RealBinDirectory {
    param([Parameter(Mandatory)][string]$Path)
    if ($Path -eq [IO.Path]::GetPathRoot($Path)) { throw "Refusing unsafe bin directory: $Path" }
    if (-not (Test-Path -LiteralPath $Path)) { [IO.Directory]::CreateDirectory($Path) | Out-Null }
    $item = Get-Item -LiteralPath $Path -Force
    if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Bin path must be a real directory, not a reparse point: $Path"
    }
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

function Get-AdapterDependencyStatus {
    param([Parameter(Mandatory)][ValidateSet('claude', 'codex')][string]$Client)
    $node = @(Get-Command node -CommandType Application -ErrorAction SilentlyContinue) | Select-Object -First 1
    if ($null -eq $node) { return '<unverified: node missing>' }
    $output = & $node.Source $targets['ops-brain-client'].Path deps-status $Client
    if ($LASTEXITCODE -ne 0) { return 'missing-or-invalid' }
    (@($output) -join '').Trim()
}

if ($Mode -eq 'Status') {
    "repo: $RepoDirectory"
    "bin: $BinDirectory"
    "node: $(Get-ApplicationPath 'node')"
    "npm: $(Get-ApplicationPath 'npm')"
    "claude: $(Get-ApplicationPath 'claude')"
    "codex (native required): $(Get-ApplicationPath 'codex.exe')"
    "claude adapter deps: $(Get-AdapterDependencyStatus 'claude')"
    "codex adapter deps: $(Get-AdapterDependencyStatus 'codex')"
    foreach ($name in $launchers) {
        $shim = Join-Path $BinDirectory "$name.cmd"
        "$name`: $(if (Test-Path -LiteralPath $shim -PathType Leaf) { $shim } else { '<missing>' })"
    }
    exit 0
}

$nodeCommand = Get-RequiredApplication 'node'
$nodeMajor = [int](& $nodeCommand.Source -p 'Number(process.versions.node.split(".")[0])')
if ($nodeMajor -lt 22) { throw 'Node.js 22 or newer is required' }

Assert-RealBinDirectory $BinDirectory
$launcherPlan = [Collections.Generic.List[object]]::new()
foreach ($name in $launchers) {
    $target = $targets[$name]
    $script = $target.Path
    if (-not (Test-Path -LiteralPath $script -PathType Leaf)) { throw "Client command is missing: $script" }
    $shim = Join-Path $BinDirectory "$name.cmd"
    $content = if ($target.Kind -eq 'node') {
        "@echo off`r`nnode `"$script`" %*`r`n"
    }
    else {
        "@echo off`r`npwsh.exe -NoLogo -NoProfile -File `"$script`" %*`r`n"
    }
    $exists = Test-Path -LiteralPath $shim
    if ($exists) {
        $shimItem = Get-Item -LiteralPath $shim -Force
        if ($shimItem.PSIsContainer -or ($shimItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Refusing to replace unsafe launcher path: $shim"
        }
        if ([IO.File]::ReadAllText($shim) -ne $content) {
            throw "Refusing to replace non-owned launcher: $shim"
        }
    }
    $launcherPlan.Add([pscustomobject]@{ Name = $name; Shim = $shim; Content = $content; Exists = $exists })
}

if (-not $SkipDependencies) {
    $claudeDependencies = (& $nodeCommand.Source $targets['ops-brain-client'].Path deps-status claude).Trim()
    $codexDependencies = (& $nodeCommand.Source $targets['ops-brain-client'].Path deps-status codex).Trim()
    if ($claudeDependencies -eq 'ready' -and $codexDependencies -eq 'ready') {
        'using ready lockfile-pinned adapter dependencies'
        $SkipDependencies = $true
    }
}

if (-not $SkipDependencies) {
    $npmCommand = Get-RequiredApplication 'npm'
    foreach ($adapter in @('claude-channel', 'codex-app-server')) {
        $adapterDirectory = Join-Path $RepoDirectory "adapters\$adapter"
        & $npmCommand.Source --prefix $adapterDirectory ci --ignore-scripts
        if ($LASTEXITCODE -ne 0) { throw "npm ci failed for $adapter" }
    }
}

foreach ($item in $launcherPlan) {
    if ($item.Exists) { continue }
    $temporary = Join-Path $BinDirectory ".$($item.Name).$([guid]::NewGuid().ToString('N')).tmp"
    try {
        [IO.File]::WriteAllText($temporary, $item.Content, [Text.UTF8Encoding]::new($false))
        Move-Item -LiteralPath $temporary -Destination $item.Shim -ErrorAction Stop
    }
    finally {
        if (Test-Path -LiteralPath $temporary -PathType Leaf) {
            Remove-Item -LiteralPath $temporary -Force
        }
    }
}

"installed ops-brain client commands in $BinDirectory"
if (-not (($env:PATH -split ';') -contains $BinDirectory)) {
    'add that directory to the user PATH before invoking the shims by name'
}
'configure profiles with ops-brain-client configure claude|codex, then run ops-brain-claude or ops-brain-codex'
