param(
    [int]$FallbackScanIntervalMs = 2000,
    [int]$DebounceMs = 150,
    [switch]$ExitAfterInitialSnapshot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public static class FlowtileObserverNative
{
    public delegate void WinEventProc(
        IntPtr hWinEventHook,
        uint eventType,
        IntPtr hwnd,
        int idObject,
        int idChild,
        uint eventThread,
        uint eventTime);

    [StructLayout(LayoutKind.Sequential)]
    public struct POINT
    {
        public int X;
        public int Y;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct MSG
    {
        public IntPtr hwnd;
        public uint message;
        public UIntPtr wParam;
        public IntPtr lParam;
        public uint time;
        public POINT pt;
        public uint lPrivate;
    }

    [DllImport("user32.dll")]
    public static extern IntPtr SetWinEventHook(
        uint eventMin,
        uint eventMax,
        IntPtr hmodWinEventProc,
        WinEventProc lpfnWinEventProc,
        uint idProcess,
        uint idThread,
        uint dwFlags);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool UnhookWinEvent(IntPtr hWinEventHook);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool PeekMessage(
        out MSG lpMsg,
        IntPtr hWnd,
        uint wMsgFilterMin,
        uint wMsgFilterMax,
        uint wRemoveMsg);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool TranslateMessage([In] ref MSG lpMsg);

    [DllImport("user32.dll")]
    public static extern IntPtr DispatchMessage([In] ref MSG lpMsg);

    [DllImport("user32.dll")]
    public static extern uint MsgWaitForMultipleObjectsEx(
        uint nCount,
        IntPtr pHandles,
        uint dwMilliseconds,
        uint dwWakeMask,
        uint dwFlags);
}
"@

$scriptDir = Split-Path -Parent $PSCommandPath
$scanScriptPath = Join-Path $scriptDir 'scan-platform.ps1'
$pwshPath = (Get-Process -Id $PID).Path

$EVENT_SYSTEM_FOREGROUND = [uint32]0x0003
$EVENT_OBJECT_CREATE = [uint32]0x8000
$EVENT_OBJECT_DESTROY = [uint32]0x8001
$EVENT_OBJECT_SHOW = [uint32]0x8002
$EVENT_OBJECT_HIDE = [uint32]0x8003
$EVENT_OBJECT_LOCATIONCHANGE = [uint32]0x800B
$OBJID_WINDOW = 0
$WINEVENT_OUTOFCONTEXT = [uint32]0x0000
$WINEVENT_SKIPOWNPROCESS = [uint32]0x0002
$PM_REMOVE = [uint32]0x0001
$QS_ALLINPUT = [uint32]0x04FF
$MWMO_INPUTAVAILABLE = [uint32]0x0004
$WAIT_TIMEOUT = [uint32]0x00000102

$script:ObserverState = [hashtable]::Synchronized(@{
    pending = $false
    pending_reason = 'initial-full-scan'
})

function Invoke-PlatformScan {
    $scanJson = & $pwshPath -NoProfile -ExecutionPolicy Bypass -File $scanScriptPath
    if ($LASTEXITCODE -ne 0) {
        throw "scan-platform exited with code $LASTEXITCODE"
    }
    if ([string]::IsNullOrWhiteSpace($scanJson)) {
        throw 'scan-platform returned empty output'
    }

    return $scanJson | ConvertFrom-Json -Depth 12
}

function Publish-ObservationWarning {
    param(
        [string]$Reason,
        [string]$Message
    )

    $json = @{
        kind = 'warning'
        reason = $Reason
        message = $Message
    } | ConvertTo-Json -Depth 6 -Compress

    [Console]::Out.WriteLine($json)
    [Console]::Out.Flush()
}

function Publish-ObservationSnapshot {
    param(
        [string]$Reason
    )

    try {
        $snapshot = Invoke-PlatformScan
        $json = @{
            kind = 'snapshot'
            reason = $Reason
            snapshot = $snapshot
        } | ConvertTo-Json -Depth 12 -Compress

        [Console]::Out.WriteLine($json)
        [Console]::Out.Flush()
    }
    catch {
        Publish-ObservationWarning -Reason $Reason -Message $_.Exception.Message
    }
}

function Mark-PendingObservation {
    param(
        [string]$Reason
    )

    $script:ObserverState.pending = $true
    $script:ObserverState.pending_reason = $Reason
}

$callback = [FlowtileObserverNative+WinEventProc]{
    param(
        [IntPtr]$Hook,
        [uint32]$EventType,
        [IntPtr]$WindowHandle,
        [int]$ObjectId,
        [int]$ChildId,
        [uint32]$EventThread,
        [uint32]$EventTime
    )

    if ($WindowHandle -eq [IntPtr]::Zero) {
        return
    }

    if ($EventType -ne $EVENT_SYSTEM_FOREGROUND -and $ObjectId -ne $OBJID_WINDOW) {
        return
    }

    $reason = switch ($EventType) {
        $EVENT_SYSTEM_FOREGROUND { 'win-event-foreground' }
        $EVENT_OBJECT_CREATE { 'win-event-create' }
        $EVENT_OBJECT_DESTROY { 'win-event-destroy' }
        $EVENT_OBJECT_SHOW { 'win-event-show' }
        $EVENT_OBJECT_HIDE { 'win-event-hide' }
        $EVENT_OBJECT_LOCATIONCHANGE { 'win-event-location-change' }
        default { 'win-event-update' }
    }

    Mark-PendingObservation -Reason $reason
}

$hooks = @()

try {
    foreach ($registration in @(
        @{ Min = $EVENT_SYSTEM_FOREGROUND; Max = $EVENT_SYSTEM_FOREGROUND },
        @{ Min = $EVENT_OBJECT_CREATE; Max = $EVENT_OBJECT_HIDE },
        @{ Min = $EVENT_OBJECT_LOCATIONCHANGE; Max = $EVENT_OBJECT_LOCATIONCHANGE }
    )) {
        $hook = [FlowtileObserverNative]::SetWinEventHook(
            [uint32]$registration.Min,
            [uint32]$registration.Max,
            [IntPtr]::Zero,
            $callback,
            [uint32]0,
            [uint32]0,
            [uint32]($WINEVENT_OUTOFCONTEXT -bor $WINEVENT_SKIPOWNPROCESS)
        )

        if ($hook -eq [IntPtr]::Zero) {
            throw "SetWinEventHook failed for range $($registration.Min)-$($registration.Max)"
        }

        $hooks += $hook
    }

    Publish-ObservationSnapshot -Reason 'initial-full-scan'
    if ($ExitAfterInitialSnapshot) {
        return
    }

    $message = New-Object FlowtileObserverNative+MSG
    $lastEmitAt = [Environment]::TickCount64
    $lastPeriodicScanAt = $lastEmitAt
    $lastLoopAt = $lastEmitAt

    while ($true) {
        $waitResult = [FlowtileObserverNative]::MsgWaitForMultipleObjectsEx(
            [uint32]0,
            [IntPtr]::Zero,
            [uint32]100,
            $QS_ALLINPUT,
            $MWMO_INPUTAVAILABLE
        )

        while ([FlowtileObserverNative]::PeekMessage([ref]$message, [IntPtr]::Zero, 0, 0, $PM_REMOVE)) {
            [void][FlowtileObserverNative]::TranslateMessage([ref]$message)
            [void][FlowtileObserverNative]::DispatchMessage([ref]$message)
        }

        $now = [Environment]::TickCount64
        if (($now - $lastLoopAt) -ge ([Math]::Max($FallbackScanIntervalMs, 1000) * 3)) {
            Publish-ObservationSnapshot -Reason 'resume-revalidation'
            $lastEmitAt = $now
            $lastPeriodicScanAt = $now
            $script:ObserverState.pending = $false
        }
        elseif ($script:ObserverState.pending -and (($now - $lastEmitAt) -ge $DebounceMs)) {
            $reason = [string]$script:ObserverState.pending_reason
            $script:ObserverState.pending = $false
            Publish-ObservationSnapshot -Reason $reason
            $lastEmitAt = $now
            $lastPeriodicScanAt = $now
        }
        elseif (($now - $lastPeriodicScanAt) -ge $FallbackScanIntervalMs) {
            Publish-ObservationSnapshot -Reason 'periodic-full-scan'
            $lastEmitAt = $now
            $lastPeriodicScanAt = $now
        }

        $lastLoopAt = $now
    }
}
finally {
    foreach ($hook in $hooks) {
        if ($hook -ne [IntPtr]::Zero) {
            [void][FlowtileObserverNative]::UnhookWinEvent($hook)
        }
    }
}
