#requires -Version 7.4
# Emit one DPAPI-protected agent token to the requesting adapter's private pipe.

[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$AgentCredentialFile,
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9._-]{1,80}$')]
    [string]$AgentName,
    [switch]$ValidateOnly
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
if ($ValidateOnly) {
    $credential = $null
    return
}

# This handle check is a guardrail against accidental console or file disclosure;
# FILE_TYPE_PIPE also includes named pipes and sockets, so it is not caller
# authentication. Check before materializing bearer plaintext.
if ($null -eq ('OpsBrainStandardHandle' -as [type])) {
    Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;

public static class OpsBrainStandardHandle {
    private const uint FileTypePipe = 3;
    private const int StandardOutputHandle = -11;

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern System.IntPtr GetStdHandle(int standardHandle);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern uint GetFileType(System.IntPtr handle);

    public static bool IsOutputPipe() {
        System.IntPtr handle = GetStdHandle(StandardOutputHandle);
        return handle != System.IntPtr.Zero
            && handle != new System.IntPtr(-1)
            && GetFileType(handle) == FileTypePipe;
    }
}
'@
}
if (-not [OpsBrainStandardHandle]::IsOutputPipe()) {
    throw 'Refusing to emit an agent token unless stdout is an operating-system pipe'
}

$standardOutput = $null
$bstr = [IntPtr]::Zero
$characters = $null
$bytes = $null
try {
    $standardOutput = [Console]::OpenStandardOutput()
    $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($credential.Password)
    $characterCount = [int]([Runtime.InteropServices.Marshal]::ReadInt32($bstr, -4) / 2)
    if ($characterCount -le 0) {
        throw 'Agent credential must contain one non-empty line'
    }
    $characters = [char[]]::new($characterCount)
    [Runtime.InteropServices.Marshal]::Copy($bstr, $characters, 0, $characterCount)
    $hasNonWhitespace = $false
    foreach ($character in $characters) {
        if ($character -eq "`n" -or $character -eq "`r" -or $character -eq [char]0) {
            throw 'Agent credential must contain one non-empty line'
        }
        if (-not [char]::IsWhiteSpace($character)) { $hasNonWhitespace = $true }
    }
    if (-not $hasNonWhitespace) { throw 'Agent credential must contain one non-empty line' }
    $bytes = [Text.Encoding]::UTF8.GetBytes([char[]]$characters)
    $standardOutput.Write($bytes, 0, $bytes.Length)
    $standardOutput.Flush()
}
finally {
    if ($null -ne $bytes) { [Array]::Clear($bytes, 0, $bytes.Length) }
    if ($null -ne $characters) { [Array]::Clear($characters, 0, $characters.Length) }
    if ($bstr -ne [IntPtr]::Zero) { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) }
    if ($null -ne $standardOutput) { $standardOutput.Dispose() }
    $bytes = $null
    $characters = $null
    $bstr = [IntPtr]::Zero
    $standardOutput = $null
    $credential = $null
}
