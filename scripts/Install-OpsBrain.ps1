#requires -Version 7.4
# Public installer name; the old implementation path remains for compatibility.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

& "$PSScriptRoot\Install-OpsBrainLive.ps1" @args
