Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$rawInput = [Console]::In.ReadToEnd()
if ([string]::IsNullOrWhiteSpace($rawInput)) {
    @{
        attempted = 0
        applied = 0
        failures = @()
    } | ConvertTo-Json -Depth 4 -Compress
    exit 0
}

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public static class FlowtileNativeApply
{
    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool SetWindowPos(
        IntPtr hWnd,
        IntPtr hWndInsertAfter,
        int X,
        int Y,
        int cx,
        int cy,
        uint uFlags);
}
"@

try {
    $payload = $rawInput | ConvertFrom-Json -Depth 8
    $operations = @($payload.operations)
    $failures = [System.Collections.Generic.List[object]]::new()
    $applied = 0
    $flags = 0x0004 -bor 0x0010 -bor 0x0200 -bor 0x0040

    foreach ($operation in $operations) {
        $ok = [FlowtileNativeApply]::SetWindowPos(
            [IntPtr][long]$operation.hwnd,
            [IntPtr]::Zero,
            [int]$operation.rect.x,
            [int]$operation.rect.y,
            [int]$operation.rect.width,
            [int]$operation.rect.height,
            [uint32]$flags
        )

        if ($ok) {
            $applied += 1
            continue
        }

        $lastError = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
        [void]$failures.Add(@{
            hwnd = [uint64]$operation.hwnd
            message = "SetWindowPos failed with Win32 error $lastError"
        })
    }

    @{
        attempted = $operations.Count
        applied = $applied
        failures = $failures
    } | ConvertTo-Json -Depth 8 -Compress
}
catch {
    Write-Error $_
    exit 1
}
