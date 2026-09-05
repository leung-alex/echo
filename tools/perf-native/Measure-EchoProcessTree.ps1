#requires -Version 7.0
<#
Read-only resource sampler for an ALREADY RUNNING isolated Echo process.
Does not start/stop processes, manipulate the clipboard, or trim working sets.
Private Bytes = private committed bytes; Private Working Set = private resident bytes.
NOT a startup timing tool; no GPU counters; discovered process set still needs independent verification.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateRange(1,2147483647)][int]$RootProcessId,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(1,86400)][int]$DurationSeconds = 300,
    [ValidateRange(500,60000)][int]$IntervalMilliseconds = 1000
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Run this collector on Windows with PowerShell 7+.' }
. (Join-Path $PSScriptRoot 'Common.ps1')
$rootInfo = Get-CimInstance Win32_Process -Filter "ProcessId = $RootProcessId"
if ($null -eq $rootInfo) { throw 'Root PID does not exist.' }
$out = New-EchoEvidenceDirectory $OutputDirectory
$rootCreated = ([datetime]$rootInfo.CreationDate).ToUniversalTime().Ticks
$logicalCpus = [Environment]::ProcessorCount
$clock = [Diagnostics.Stopwatch]::StartNew()
$known = @{}             # PID -> identity, never trusts PID alone.
$known[$RootProcessId] = @{ created=$rootCreated; parent=0; name=$rootInfo.Name }
$previousCpu = @{}
$allIdentities = @{}
$errors = [Collections.Generic.List[string]]::new()
$aggregatePath = Join-Path $out 'aggregate.csv'
$processPath = Join-Path $out 'processes.csv'
$sampleIndex = 0
$validSamples = 0
$previousElapsed = 0.0
$rootEverExited = $false
$completed = $false
$beginUtc = [datetime]::UtcNow.ToString('o')
try {
    while ($clock.Elapsed.TotalSeconds -lt $DurationSeconds) {
        $tickStart = $clock.Elapsed.TotalMilliseconds
        $elapsed = $clock.Elapsed.TotalSeconds
        $snapshot = @(Get-CimInstance Win32_Process)
        $current = @{}
        foreach ($record in $snapshot) {
            $processIdValue = [int]$record.ProcessId
            $current[$processIdValue] = $record
        }
        $topologyChanged = $false
        # Remove dead/reused PID identities before looking at parent links.
        foreach ($processIdValue in @($known.Keys)) {
            $record = $current[$processIdValue]
            if ($null -eq $record -or ([datetime]$record.CreationDate).ToUniversalTime().Ticks -ne $known[$processIdValue].created) {
                if ($processIdValue -eq $RootProcessId) { $rootEverExited = $true }
                $topologyChanged = $true
                $known.Remove($processIdValue)
                $previousCpu.Remove($processIdValue)
            }
        }
        # Repeated expansion discovers multiple descendant generations in one snapshot.
        do {
            $added = $false
            foreach ($record in $snapshot) {
                $processIdValue = [int]$record.ProcessId
                $parentIdValue = [int]$record.ParentProcessId
                if (-not $known.ContainsKey($processIdValue) -and $known.ContainsKey($parentIdValue)) {
                    $created = ([datetime]$record.CreationDate).ToUniversalTime().Ticks
                    if ($created -ge $known[$parentIdValue].created) {
                        $known[$processIdValue] = @{ created=$created; parent=$parentIdValue; name=$record.Name }
                        $added = $true
                        $topologyChanged = $true
                    }
                }
            }
        } while ($added)
        if ($known.Count -eq 0) { $errors.Add("sample=$sampleIndex no live owned processes"); break }
        $privateWs = @{}
        try {
            foreach ($counter in @(Get-CimInstance Win32_PerfRawData_PerfProc_Process)) {
                if ($null -ne $counter.PSObject.Properties['WorkingSetPrivate']) {
                    $privateWs[[int]$counter.IDProcess] = [double]$counter.WorkingSetPrivate
                }
            }
        } catch { $errors.Add("sample=$sampleIndex private-working-set counters unavailable") }
        $sumPrivate = 0.0; $sumPrivateWs = 0.0; $sumWs = 0.0; $sumCpu = 0.0
        $sumHandles = 0; $sumThreads = 0; $readCount = 0
        $valid = (-not $rootEverExited); $cpuValid = ($sampleIndex -gt 0 -and -not $topologyChanged)
        foreach ($processIdValue in @($known.Keys | Sort-Object)) {
            $identity = $known[$processIdValue]
            $key = "$processIdValue`:$($identity.created)"
            $allIdentities[$key] = @{ process_id=$processIdValue; created_utc_ticks=$identity.created; parent_id=$identity.parent; name=$identity.name }
            try {
                $process = Get-Process -Id $processIdValue -ErrorAction Stop
                try {
                    $process.Refresh()
                    # CIM timestamps can be rounded to microseconds; allow at most 1ms conversion difference.
                    if ([Math]::Abs($process.StartTime.ToUniversalTime().Ticks - [long]$identity.created) -gt 10000) { throw 'PID identity changed during sampling.' }
                    $bytes = [double]$process.PrivateMemorySize64
                    $ws = [double]$process.WorkingSet64
                    $cpuMs = $process.TotalProcessorTime.TotalMilliseconds
                    $handles = $process.HandleCount
                    $threads = $process.Threads.Count
                    $pws = if ($privateWs.ContainsKey($processIdValue)) { $privateWs[$processIdValue] } else { $null }
                    if ($null -eq $pws) { $valid = $false }
                    if ($previousCpu.ContainsKey($processIdValue)) {
                        $delta = $cpuMs - [double]$previousCpu[$processIdValue]
                        if ($delta -lt 0) { $cpuValid=$false } else { $sumCpu += $delta }
                    } else { $cpuValid=$false }
                    $previousCpu[$processIdValue]=$cpuMs
                    $sumPrivate += $bytes; $sumWs += $ws
                    if ($null -ne $pws) { $sumPrivateWs += $pws }
                    $sumHandles += $handles; $sumThreads += $threads; $readCount++
                    [pscustomobject]@{ sample=$sampleIndex; elapsed_s=$elapsed; process_id=$processIdValue; created_utc_ticks=$identity.created; name=$identity.name; private_bytes=$bytes; private_working_set_bytes=$pws; working_set_bytes=$ws; cpu_total_ms=$cpuMs; handles=$handles; threads=$threads; valid=($null -ne $pws) } |
                        Export-Csv -LiteralPath $processPath -Append -NoTypeInformation -Encoding utf8
                } finally { $process.Dispose() }
            } catch { $valid=$false; $cpuValid=$false; $errors.Add("sample=$sampleIndex pid=$processIdValue metric read invalid") }
        }
        if ($readCount -ne $known.Count) { $valid=$false; $cpuValid=$false }
        $intervalSeconds = $elapsed - $previousElapsed
        $oneCore = if ($cpuValid -and $intervalSeconds -gt 0) { $sumCpu / ($intervalSeconds * 1000) * 100 } else { $null }
        $normalized = if ($null -ne $oneCore) { $oneCore / $logicalCpus } else { $null }
        $readDurationMs = $clock.Elapsed.TotalMilliseconds - $tickStart
        [pscustomobject]@{
            sample=$sampleIndex; elapsed_s=$elapsed; collection_duration_ms=$readDurationMs
            discovered_processes=$known.Count; processes_read=$readCount; valid=$valid
            private_bytes=$(if ($valid) { $sumPrivate } else { $null })
            private_working_set_bytes=$(if ($valid) { $sumPrivateWs } else { $null })
            working_set_sum_bytes=$(if ($valid) { $sumWs } else { $null })
            cpu_one_core_percent=$oneCore; cpu_system_normalized_percent=$normalized
            handles=$(if ($valid) { $sumHandles } else { $null })
            threads=$(if ($valid) { $sumThreads } else { $null })
        } | Export-Csv -LiteralPath $aggregatePath -Append -NoTypeInformation -Encoding utf8
        if ($valid) { $validSamples++ }
        $previousElapsed=$elapsed; $sampleIndex++
        $remaining = $IntervalMilliseconds - ($clock.Elapsed.TotalMilliseconds - $tickStart)
        if ($remaining -gt 0) { Start-Sleep -Milliseconds ([int]$remaining) }
    }
    $completed = ($clock.Elapsed.TotalSeconds -ge $DurationSeconds)
} finally {
    $clock.Stop()
    $metadata = [ordered]@{
        schema='echo.process.samples.v1'; status=$(if ($completed -and $validSamples -eq $sampleIndex -and $sampleIndex -gt 0 -and -not $rootEverExited) { 'SAMPLED_UNVERIFIED' } else { 'INCOMPLETE' })
        started_at_utc=$beginUtc; root_process_id=$RootProcessId; root_created_utc_ticks=$rootCreated
        requested_duration_s=$DurationSeconds; actual_duration_s=$clock.Elapsed.TotalSeconds
        interval_ms=$IntervalMilliseconds; sample_count=$sampleIndex; valid_sample_count=$validSamples
        logical_processors=$logicalCpus; root_exited=$rootEverExited
        process_set_verified=$false; gpu='NOT_RUN'; startup_ui_ready='NOT_MEASURED'
        identities=@($allIdentities.Values); errors=@($errors.ToArray())
        notes=@('Private Bytes is commit, not resident RAM.','Working-set sum may double-count shared pages.','Polling can miss short-lived or initially orphaned descendants: independently verify WebView2 process ownership.','First/new-process CPU intervals are invalid, not zero.','CIM and per-process reads are sequential, not a single atomic snapshot; collection_duration_ms records sampling cost.','Collect observer overhead separately; this tool does not certify timing accuracy or GUI readiness.')
    }
    Write-EchoJsonNew (Join-Path $out 'metadata.json') $metadata
}
Write-Output $out
