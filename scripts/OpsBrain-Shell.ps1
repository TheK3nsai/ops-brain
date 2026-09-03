#requires -Version 7.4
# ops-brain main-launcher integration for interactive PowerShell sessions.
#
# Dot-source this file from $PROFILE:
#
#   . "<client-root>\scripts\OpsBrain-Shell.ps1"
#
# (the installer prints the exact line for its checkout or bundle). It defines
# `claude` and `codex` functions that route an attended console launch through
# the ops-brain launchers in -Mode Auto. Everything else -- headless
# `claude -p`, `codex exec`, subcommands, redirected sessions, scheduled tasks,
# wake shims -- reaches the real executables untouched: functions are not
# inherited by child processes, and Auto mode passes those shapes through
# anyway.
#
# The functions carry no credential. The launchers read the protected DPAPI
# credential themselves; nothing here touches the environment.
#
# The launchers run in-process rather than through the .cmd shims: cmd.exe
# re-parses %* (an unquoted & or | splits the command line), and pwsh -File
# binds a client's -p to the launcher's -ProfileFile by prefix. Passing the
# client arguments as one array through -ClaudeArgs/-CodexArgs avoids both.
#
# Opt out for one launch:  claude --no-live      (or OPS_BRAIN_LIVE=off in the environment)
# Bypass the function:     & (Get-Command claude -CommandType Application) ...

# Interactive console sessions only. A redirected handle, a non-console host,
# or pwsh -NonInteractive means a script or a scheduled task is dot-sourcing
# the profile; those must see the real executables by name.
if ($Host.Name -ne 'ConsoleHost' -or [Console]::IsInputRedirected -or [Console]::IsOutputRedirected) { return }
if (@([Environment]::GetCommandLineArgs()) -match '^-noni') { return }

$Global:OpsBrainLaunchers = @{
    claude = Join-Path $PSScriptRoot 'ops-brain-claude-live.ps1'
    codex  = Join-Path $PSScriptRoot 'ops-brain-codex-live.ps1'
}

function Global:claude {
    if (Test-Path -LiteralPath $Global:OpsBrainLaunchers.claude -PathType Leaf) {
        & $Global:OpsBrainLaunchers.claude -Mode Auto -ClaudeArgs $args
    }
    else {
        $client = @(Get-Command claude -CommandType Application -ErrorAction SilentlyContinue) | Select-Object -First 1
        if ($null -eq $client) { throw 'claude is not installed' }
        & $client.Source @args
    }
}

function Global:codex {
    if (Test-Path -LiteralPath $Global:OpsBrainLaunchers.codex -PathType Leaf) {
        & $Global:OpsBrainLaunchers.codex -Mode Auto -CodexArgs $args
    }
    else {
        $client = @(Get-Command codex -CommandType Application -ErrorAction SilentlyContinue) | Select-Object -First 1
        if ($null -eq $client) { throw 'codex is not installed' }
        & $client.Source @args
    }
}
