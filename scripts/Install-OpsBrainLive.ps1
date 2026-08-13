#requires -Version 7.2
# Install private ops-brain live launch shims for one Windows user.

[CmdletBinding()]
param(
    [ValidateSet('Install', 'Status')]
    [string]$Mode = 'Install',
    [string]$BinDirectory = $(Join-Path $env:LOCALAPPDATA 'Programs\ops-brain-live'),
    [switch]$SkipDependencies
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$RepoDirectory = Split-Path -Parent $PSScriptRoot
$BinDirectory = [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($BinDirectory))
$launchers = @('ops-brain-claude-live', 'ops-brain-codex-live')

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

if ($Mode -eq 'Status') {
    "repo: $RepoDirectory"
    "bin: $BinDirectory"
    "node: $(Get-ApplicationPath 'node.exe')"
    "npm: $(Get-ApplicationPath 'npm.cmd')"
    "claude: $(Get-ApplicationPath 'claude.exe')"
    "codex: $(Get-ApplicationPath 'codex.exe')"
    foreach ($name in $launchers) {
        $shim = Join-Path $BinDirectory "$name.cmd"
        "$name`: $(if (Test-Path -LiteralPath $shim -PathType Leaf) { $shim } else { '<missing>' })"
    }
    exit 0
}

$nodeCommand = Get-RequiredApplication 'node.exe'
$npmCommand = Get-RequiredApplication 'npm.cmd'
$nodeMajor = [int](& $nodeCommand.Source -p 'Number(process.versions.node.split(".")[0])')
if ($nodeMajor -lt 22) { throw 'Node.js 22 or newer is required' }

Assert-RealBinDirectory $BinDirectory
$launcherPlan = [Collections.Generic.List[object]]::new()
foreach ($name in $launchers) {
    $script = Join-Path $PSScriptRoot "$name.ps1"
    if (-not (Test-Path -LiteralPath $script -PathType Leaf)) { throw "Launcher is missing: $script" }
    $shim = Join-Path $BinDirectory "$name.cmd"
    $content = "@echo off`r`npwsh.exe -NoLogo -NoProfile -File `"$script`" %*`r`n"
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

"installed live launch shims in $BinDirectory"
if (-not (($env:PATH -split ';') -contains $BinDirectory)) {
    'add that directory to the user PATH before invoking the shims by name'
}
'pass -LiveUrl, -AgentName, and the exact per-agent -AgentCredentialFile on every launch'
