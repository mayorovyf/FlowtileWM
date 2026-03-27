param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("event-stream", "sampling")]
    [string]$Mode,
    [Parameter(Mandatory = $true)]
    [string]$CliPath,
    [Parameter(Mandatory = $true)]
    [string]$RepoRoot,
    [Parameter(Mandatory = $true)]
    [int]$DaemonPid,
    [int]$HelperPid = 0,
    [Parameter(Mandatory = $true)]
    [string]$CollectorLogPath,
    [string]$EventsLogPath,
    [string]$ProcessLogPath,
    [string]$DiagnosticsLogPath,
    [string]$PerfLogPath,
    [string]$StateLogPath,
    [ValidateRange(500, 60000)]
    [int]$SampleIntervalMs = 2000
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
Set-Location $RepoRoot
$collectorWorkDir = Join-Path ([System.IO.Path]::GetDirectoryName($CollectorLogPath)) "collector-tmp"
New-Item -ItemType Directory -Path $collectorWorkDir -Force | Out-Null

function Write-CollectorLog {
    param([string]$Message)

    $timestamp = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    Add-Content -LiteralPath $CollectorLogPath -Value "[$timestamp] $Mode $Message"
}

function Append-JsonLine {
    param(
        [string]$Path,
        [object]$Value
    )

    $json = $Value | ConvertTo-Json -Depth 32 -Compress
    Add-Content -LiteralPath $Path -Value $json
}

function Test-DaemonAlive {
    return $null -ne (Get-Process -Id $DaemonPid -ErrorAction SilentlyContinue)
}

function New-CollectorTempPath {
    param(
        [string]$Prefix,
        [string]$Extension = ".log"
    )

    $name = "{0}-{1}{2}" -f $Prefix, ([guid]::NewGuid().ToString("N")), $Extension
    return Join-Path $collectorWorkDir $name
}

function Join-CommandOutput {
    param([string]$Path)

    if (-not (Test-Path $Path)) {
        return ""
    }

    $lines = Get-Content -LiteralPath $Path -ErrorAction SilentlyContinue
    if ($null -eq $lines) {
        return ""
    }

    return [string]::Join([Environment]::NewLine, @($lines))
}

function Invoke-CliJsonCommand {
    param(
        [string[]]$Arguments,
        [int]$TimeoutMs = 5000
    )

    $stdoutPath = New-CollectorTempPath -Prefix "stdout" -Extension ".json"
    $stderrPath = New-CollectorTempPath -Prefix "stderr" -Extension ".log"
    $process = $null

    try {
        $process = Start-Process `
            -FilePath $CliPath `
            -ArgumentList $Arguments `
            -WorkingDirectory $RepoRoot `
            -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath `
            -WindowStyle Hidden `
            -PassThru

        if (-not $process.WaitForExit($TimeoutMs)) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            return @{
                ok = $false
                reason = "timeout"
                exit_code = $null
                stdout = Join-CommandOutput -Path $stdoutPath
                stderr = Join-CommandOutput -Path $stderrPath
            }
        }

        $process.Refresh()
        $stdout = Join-CommandOutput -Path $stdoutPath
        $stderr = Join-CommandOutput -Path $stderrPath
        $exitCode = $process.ExitCode
        if ($exitCode -ne 0) {
            return @{
                ok = $false
                reason = "exit"
                exit_code = $exitCode
                stdout = $stdout
                stderr = $stderr
            }
        }

        if ([string]::IsNullOrWhiteSpace($stdout)) {
            return @{
                ok = $false
                reason = "empty"
                exit_code = $exitCode
                stdout = $stdout
                stderr = $stderr
            }
        }

        try {
            return @{
                ok = $true
                reason = "ok"
                exit_code = $exitCode
                stdout = $stdout
                stderr = $stderr
                json = ($stdout | ConvertFrom-Json -Depth 64)
            }
        }
        catch {
            return @{
                ok = $false
                reason = "json"
                exit_code = $exitCode
                stdout = $stdout
                stderr = $stderr
                parse_error = $_.Exception.Message
            }
        }
    }
    finally {
        if ($null -ne $process -and -not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
        Remove-Item $stdoutPath, $stderrPath -ErrorAction SilentlyContinue
    }
}

try {
    if ($Mode -eq "event-stream") {
        if ([string]::IsNullOrWhiteSpace($EventsLogPath)) {
            throw "event-stream mode requires EventsLogPath"
        }

        $eventsParent = [System.IO.Path]::GetDirectoryName($EventsLogPath)
        if (-not [string]::IsNullOrWhiteSpace($eventsParent)) {
            New-Item -ItemType Directory -Path $eventsParent -Force | Out-Null
        }

        Write-CollectorLog "collector-started daemon_pid=$DaemonPid"
        while (Test-DaemonAlive) {
            try {
                Write-CollectorLog "connect-attempt"
                & $CliPath events 2>> $CollectorLogPath | ForEach-Object {
                    if (-not [string]::IsNullOrWhiteSpace($_)) {
                        Add-Content -LiteralPath $EventsLogPath -Value $_
                    }
                }
                $exitCode = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
                Write-CollectorLog "stream-finished exit_code=$exitCode"
            }
            catch {
                Write-CollectorLog "stream-error $($_.Exception.Message)"
            }

            if (-not (Test-DaemonAlive)) {
                break
            }

            Start-Sleep -Milliseconds 750
        }

        Write-CollectorLog "collector-stopped"
        return
    }

    $logicalCpuCount = [Math]::Max([Environment]::ProcessorCount, 1)
    $previousCpuByPid = @{}
    $lastSampleAt = $null
    $lastStateVersion = $null

    function Get-ProcessSample {
        param(
            [int]$ProcessId,
            [string]$Role,
            [double]$IntervalSeconds
        )

        $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
        if ($null -eq $process) {
            return @{
                role = $Role
                pid = $ProcessId
                running = $false
            }
        }

        $cpuTotal = if ($null -ne $process.CPU) { [double]$process.CPU } else { 0.0 }
        $cpuPercent = $null
        if ($previousCpuByPid.ContainsKey($ProcessId) -and $IntervalSeconds -gt 0) {
            $deltaCpu = [Math]::Max(0.0, $cpuTotal - $previousCpuByPid[$ProcessId])
            $cpuPercent = [Math]::Round(($deltaCpu / ($IntervalSeconds * $logicalCpuCount)) * 100.0, 2)
        }
        $previousCpuByPid[$ProcessId] = $cpuTotal

        return @{
            role = $Role
            pid = $ProcessId
            running = $true
            process_name = $process.ProcessName
            cpu_total_s = [Math]::Round($cpuTotal, 3)
            cpu_percent = $cpuPercent
            working_set_mb = [Math]::Round($process.WorkingSet64 / 1MB, 2)
            private_mb = [Math]::Round($process.PrivateMemorySize64 / 1MB, 2)
            handles = $process.Handles
            thread_count = $process.Threads.Count
        }
    }

    Write-CollectorLog "collector-started daemon_pid=$DaemonPid helper_pid=$HelperPid"

    while (Test-DaemonAlive) {
        $sampleTime = Get-Date
        $timestamp = $sampleTime.ToString("o")
        $intervalSeconds = if ($null -ne $lastSampleAt) {
            ($sampleTime - $lastSampleAt).TotalSeconds
        } else {
            0.0
        }
        $lastSampleAt = $sampleTime

        $processes = @(
            Get-ProcessSample -ProcessId $DaemonPid -Role "daemon" -IntervalSeconds $intervalSeconds
        )
        if ($HelperPid -gt 0) {
            $processes += Get-ProcessSample -ProcessId $HelperPid -Role "touchpad-helper" -IntervalSeconds $intervalSeconds
        }

        Append-JsonLine -Path $ProcessLogPath -Value @{
            timestamp = $timestamp
            sample_interval_ms = $SampleIntervalMs
            processes = $processes
        }

        try {
            $diagnosticsResult = Invoke-CliJsonCommand -Arguments @("dump-diagnostics") -TimeoutMs 5000
            if ($diagnosticsResult.ok) {
                $diagnostics = $diagnosticsResult.json
                Append-JsonLine -Path $DiagnosticsLogPath -Value @{
                    timestamp = $timestamp
                    sample = $diagnostics
                }

                $stateVersion = $diagnostics.state_version
                $perfMetrics = @()
                $perfProperty = $null
                if ($null -ne $diagnostics.diagnostics) {
                    $perfProperty = $diagnostics.diagnostics.PSObject.Properties["perf"]
                }
                if ($null -ne $perfProperty -and $null -ne $diagnostics.diagnostics.perf) {
                    $perfMetrics = @($diagnostics.diagnostics.perf.metrics) |
                        Sort-Object -Property total_duration_us -Descending
                }

                Append-JsonLine -Path $PerfLogPath -Value @{
                    timestamp = $timestamp
                    state_version = $stateVersion
                    top_metrics = @($perfMetrics | Select-Object -First 8)
                }

                if ($stateVersion -ne $lastStateVersion) {
                    $snapshotResult = Invoke-CliJsonCommand -Arguments @("snapshot") -TimeoutMs 5000
                    if ($snapshotResult.ok) {
                        $snapshot = $snapshotResult.json
                        Append-JsonLine -Path $StateLogPath -Value @{
                            timestamp = $timestamp
                            state_version = $stateVersion
                            snapshot = $snapshot
                        }
                        $lastStateVersion = $stateVersion
                    }
                    else {
                        $detail = if (-not [string]::IsNullOrWhiteSpace($snapshotResult.stderr)) {
                            " stderr=$($snapshotResult.stderr)"
                        } elseif (-not [string]::IsNullOrWhiteSpace($snapshotResult.parse_error)) {
                            " parse_error=$($snapshotResult.parse_error)"
                        } else {
                            ""
                        }
                        Write-CollectorLog "snapshot unavailable reason=$($snapshotResult.reason) exit_code=$($snapshotResult.exit_code)$detail"
                    }
                }
            }
            else {
                $detail = if (-not [string]::IsNullOrWhiteSpace($diagnosticsResult.stderr)) {
                    " stderr=$($diagnosticsResult.stderr)"
                } elseif (-not [string]::IsNullOrWhiteSpace($diagnosticsResult.parse_error)) {
                    " parse_error=$($diagnosticsResult.parse_error)"
                } else {
                    ""
                }
                Write-CollectorLog "dump-diagnostics unavailable reason=$($diagnosticsResult.reason) exit_code=$($diagnosticsResult.exit_code)$detail"
            }
        }
        catch {
            Write-CollectorLog "sampling-error $($_.Exception.Message)"
        }

        Start-Sleep -Milliseconds $SampleIntervalMs
    }

    Write-CollectorLog "collector-stopped"
}
catch {
    Write-CollectorLog "fatal $($_.Exception.Message)"
    throw
}
