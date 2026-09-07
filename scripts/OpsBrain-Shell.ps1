#requires -Version 7.4
# Explicit ops-brain live commands for interactive PowerShell sessions.
#
# Dot-source this file from $PROFILE:
#
#   . "<client-root>\scripts\OpsBrain-Shell.ps1"
#
# (the installer prints the exact line for its checkout or bundle). It defines
# `ops-brain-claude` and `ops-brain-codex` functions that launch live sessions.
# Plain `claude` and `codex` keep their existing command resolution.
#
# The functions carry no credential. The launchers read the protected DPAPI
# credential themselves; nothing here touches the environment.
#
# The launchers run in-process rather than through the .cmd shims: cmd.exe
# re-parses %* (an unquoted & or | splits the command line), and pwsh -File
# binds a client's -p to the launcher's -ProfileFile by prefix. Passing the
# client arguments as one array through -ClaudeArgs/-CodexArgs avoids both.
#
# Guard a dash argument:   ops-brain-claude '--' --literal-value
#
# That last one is a PowerShell parser rule, not a launcher choice: the parser
# consumes the first unquoted `--` in argument mode before $args is populated,
# so an unquoted `ops-brain-claude -- --literal-value` arrives here as `--literal-value`
# and the end-of-options guard is silently lost. Nothing inside the function
# can recover it (a ValueFromRemainingArguments parameter does not help; the
# token is gone before binding). Quoting it preserves it. Pinned by the `112`
# assertion in Test-OpsBrainLiveWindows.ps1.

# Interactive console sessions only. A redirected handle, a non-console host,
# or pwsh -NonInteractive means a script or a scheduled task is dot-sourcing
# the profile; those must see the real executables by name.
if ($Host.Name -ne 'ConsoleHost' -or [Console]::IsInputRedirected -or [Console]::IsOutputRedirected) { return }
if (@([Environment]::GetCommandLineArgs()) -match '^-noni') { return }

$Global:OpsBrainLaunchers = @{
    claude = Join-Path $PSScriptRoot 'ops-brain-claude-live.ps1'
    codex  = Join-Path $PSScriptRoot 'ops-brain-codex-live.ps1'
}

function Global:ops-brain-claude {
    & $Global:OpsBrainLaunchers.claude -Mode Run -ClaudeArgs $args
}

function Global:ops-brain-codex {
    & $Global:OpsBrainLaunchers.codex -Mode Run -CodexArgs $args
}
