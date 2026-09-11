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
    [ValidateRange(100,60000)][int]$IntervalMilliseconds = 1000,
    [string[]]$ExpectedExecutable = @()
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Run this collector on Windows with PowerShell 7+.' }
. (Join-Path $PSScriptRoot 'Common.ps1')
Add-Type -Path (Join-Path $PSScriptRoot 'ProcessSnapshot.cs')
$rootInfo = [Echo.Performance.ProcessSnapshot]::Capture() | Where-Object ProcessId -EQ $RootProcessId
if ($null -eq $rootInfo) { throw 'Root PID does not exist.' }
$out = New-EchoEvidenceDirectory $OutputDirectory
$rootCreated = ([datetime]$rootInfo.CreationDate).ToUniversalTime().Ticks
$rootSession = $rootInfo.SessionId
if ($rootInfo.IdentityError -ne 0 -or $rootSession -lt 0) { throw 'Root process identity could not be established.' }
$expectedPaths = @($ExpectedExecutable | ForEach-Object { (Resolve-Path -LiteralPath $_).Path })
if ($expectedPaths.Count -gt 0 -and $rootInfo.ExecutablePath -notin $expectedPaths) {
    throw 'Root executable does not match the measured product allowlist.'
}
$logicalCpus = [Environment]::ProcessorCount
# Initialize native and managed Process counters outside the timed window.
# The warm-up is not a sample and cannot contribute a fabricated zero/short read.
$null = [Echo.Performance.ProcessSnapshot]::Capture()
$null = [Echo.Performance.ProcessSnapshot]::ReadMemory($RootProcessId, $rootCreated)
$warmProcess = Get-Process -Id $RootProcessId -ErrorAction Stop
try {
    $warmProcess.Refresh()
    $null = $warmProcess.StartTime
    $null = $warmProcess.TotalProcessorTime
    $null = $warmProcess.PrivateMemorySize64
    $null = $warmProcess.WorkingSet64
    $null = $warmProcess.HandleCount
    $null = $warmProcess.Threads.Count
} finally { $warmProcess.Dispose() }
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
$tracePrefix = 'echo-memory-' + [guid]::NewGuid().ToString('N')
$traceSources = @()
$lifetimeEvents = [Collections.Generic.List[object]]::new()
$traceError = $null
try {
    # Independent OS lifecycle stream complements sampled ancestry; never upgrades ownership by itself.
    foreach ($kind in @('Start', 'Stop')) {
        $source = "$tracePrefix-$kind"
        try {
            Register-CimIndicationEvent -Namespace root/cimv2 -Query "SELECT * FROM Win32_Process${kind}Trace" -SourceIdentifier $source -ErrorAction Stop | Out-Null
            $traceSources += $source
        } catch {
            $traceError = $_.Exception.Message
            break
        }
    }
    $clock.Restart()
    $beginUtc = [datetime]::UtcNow.ToString('o')
    while ($clock.Elapsed.TotalSeconds -lt $DurationSeconds) {
        $tickStart = $clock.Elapsed.TotalMilliseconds
        $elapsed = $clock.Elapsed.TotalSeconds
        $snapshot = @([Echo.Performance.ProcessSnapshot]::Capture())
        foreach ($source in $traceSources) {
            foreach ($trace in @(Get-Event -SourceIdentifier $source -ErrorAction SilentlyContinue)) {
                $record = $trace.SourceEventArgs.NewEvent
                $lifetimeEvents.Add([ordered]@{
                    kind=$(if ($source.EndsWith('-Start')) { 'start' } else { 'stop' })
                    process_id=[int]$record.ProcessID; name=[string]$record.ProcessName
                    observed_utc=$trace.TimeGenerated.ToUniversalTime().ToString('o')
                    os_time=[string]$record.TIME_CREATED
                })
                Remove-Event -EventIdentifier $trace.EventIdentifier
            }
        }
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
        # Exact run-specific executable copies also reveal products detached from their parent.
        foreach ($record in $snapshot) {
            $processIdValue = [int]$record.ProcessId
            if ($record.SessionId -eq $rootSession -and $expectedPaths.Count -gt 0 -and $record.ExecutablePath -in $expectedPaths -and -not $known.ContainsKey($processIdValue)) {
                $known[$processIdValue] = @{ created=([datetime]$record.CreationDate).ToUniversalTime().Ticks; parent=[int]$record.ParentProcessId; name=$record.Name; detached_unverified=$true }
                $topologyChanged = $true
            }
        }
        if ($known.Count -eq 0) { $errors.Add("sample=$sampleIndex no live owned processes"); break }
        $sumPrivate = 0.0; $sumPrivateWs = 0.0; $sumWs = 0.0; $sumCpu = 0.0
        $sumHandles = 0; $sumThreads = 0; $readCount = 0
        $valid = (-not $rootEverExited); $cpuValid = ($sampleIndex -gt 0 -and -not $topologyChanged)
        $expectedNames = @($expectedPaths | ForEach-Object { [IO.Path]::GetFileName($_) })
        foreach ($record in $snapshot) {
            if ($record.SessionId -eq $rootSession -and $record.Name -in $expectedNames -and $record.IdentityError -ne 0) {
                $valid=$false; $errors.Add("sample=$sampleIndex unresolved product identity pid=$($record.ProcessId)")
            }
        }
        foreach ($processIdValue in @($known.Keys | Sort-Object)) {
            $identity = $known[$processIdValue]
            $key = "$processIdValue`:$($identity.created)"
            $allIdentities[$key] = @{ process_id=$processIdValue; created_utc_ticks=$identity.created; parent_id=$identity.parent; name=$identity.name; session_id=$current[$processIdValue].SessionId; executable=$current[$processIdValue].ExecutablePath }
            if ($identity.created -lt $rootCreated -or $current[$processIdValue].IdentityError -ne 0 -or $current[$processIdValue].SessionId -ne $rootSession -or $identity.ContainsKey('detached_unverified')) {
                $valid=$false; $errors.Add("sample=$sampleIndex unexpected or unresolved run identity pid=$processIdValue")
            }
            try {
                $process = Get-Process -Id $processIdValue -ErrorAction Stop
                try {
                    $process.Refresh()
                    # Both native and managed timestamps use the same Windows process creation time.
                    if ($process.StartTime.ToUniversalTime().Ticks -ne [long]$identity.created) { throw 'PID identity changed during sampling.' }
                    $memory = [Echo.Performance.ProcessSnapshot]::ReadMemory($processIdValue, [long]$identity.created)
                    $bytes = [double]$memory.PrivateUsage.ToUInt64()
                    $ws = [double]$memory.WorkingSet.ToUInt64()
                    $cpuMs = $process.TotalProcessorTime.TotalMilliseconds
                    $handles = $process.HandleCount
                    $threads = $process.Threads.Count
                    $pws = [double]$memory.PrivateWorkingSet.ToUInt64()
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
    foreach ($source in $traceSources) {
        Unregister-Event -SourceIdentifier $source -ErrorAction SilentlyContinue
        foreach ($trace in @(Get-Event -SourceIdentifier $source -ErrorAction SilentlyContinue)) {
            $record = $trace.SourceEventArgs.NewEvent
            $lifetimeEvents.Add([ordered]@{kind=$(if ($source.EndsWith('-Start')) {'start'} else {'stop'}); process_id=[int]$record.ProcessID; name=[string]$record.ProcessName; observed_utc=$trace.TimeGenerated.ToUniversalTime().ToString('o'); os_time=[string]$record.TIME_CREATED})
            Remove-Event -EventIdentifier $trace.EventIdentifier
        }
    }
    Write-EchoJsonNew (Join-Path $out 'lifetime-events.json') ([ordered]@{schema='echo.process.lifetime.v1'; status=$(if ($null -eq $traceError) {'REVIEW_REQUIRED'} else {'UNAVAILABLE'}); error=$traceError; events=@($lifetimeEvents.ToArray()); expected_executables=$expectedPaths})
    $clock.Stop()
    $metadata = [ordered]@{
        schema='echo.process.samples.v1'; status=$(if ($completed -and $validSamples -eq $sampleIndex -and $sampleIndex -gt 0 -and -not $rootEverExited) { 'SAMPLED_UNVERIFIED' } else { 'INCOMPLETE' })
        started_at_utc=$beginUtc; root_process_id=$RootProcessId; root_created_utc_ticks=$rootCreated; root_session_id=$rootSession
        native_counter_sha256=(Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'ProcessSnapshot.cs') -Algorithm SHA256).Hash.ToLowerInvariant()
        requested_duration_s=$DurationSeconds; actual_duration_s=$clock.Elapsed.TotalSeconds
        interval_ms=$IntervalMilliseconds; sample_count=$sampleIndex; valid_sample_count=$validSamples
        logical_processors=$logicalCpus; root_exited=$rootEverExited
        process_set_verified=$false; gpu='NOT_RUN'; startup_ui_ready='NOT_MEASURED'
        identities=@($allIdentities.Values); errors=@($errors.ToArray())
        notes=@('Private Bytes is commit, not resident RAM.','Working-set sum may double-count shared pages.','Review OS lifetime events for short-lived helpers and reconcile exact executable identities; lifecycle events do not measure between-sample peaks.','First/new-process CPU intervals are invalid, not zero.','Toolhelp identity and per-process reads are sequential, not a single atomic snapshot; collection_duration_ms records sampling cost.','Collect observer overhead separately; this tool does not certify timing accuracy or GUI readiness.')
    }
    Write-EchoJsonNew (Join-Path $out 'metadata.json') $metadata
}
Write-Output $out
