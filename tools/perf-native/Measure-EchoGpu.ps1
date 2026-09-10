#requires -Version 7.0
# Separate observer: slow GPU counter providers cannot delay Private Bytes sampling.
[CmdletBinding()]
param(
 [Parameter(Mandatory)][int]$RootProcessId,
 [Parameter(Mandatory)][string[]]$ExpectedExecutable,
 [Parameter(Mandatory)][string]$OutputDirectory,
 [ValidateRange(1,3600)][int]$DurationSeconds=345
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'Common.ps1')
Add-Type -Path (Join-Path $PSScriptRoot 'ProcessSnapshot.cs')
$out=New-EchoEvidenceDirectory $OutputDirectory
$paths=@($ExpectedExecutable | ForEach-Object {(Resolve-Path -LiteralPath $_).Path})
$root=[Echo.Performance.ProcessSnapshot]::Capture() | Where-Object ProcessId -EQ $RootProcessId
if ($null -eq $root -or $root.IdentityError -ne 0 -or $root.ExecutablePath -notin $paths) {throw 'Cannot establish GPU observer root identity'}
$clock=[Diagnostics.Stopwatch]::StartNew()
$samples=0
$failure=$null
try {
 while ($clock.Elapsed.TotalSeconds -lt $DurationSeconds) {
  $tick=$clock.Elapsed.TotalMilliseconds
  $snapshot=[Echo.Performance.ProcessSnapshot]::Capture()
  $live=$snapshot | Where-Object ProcessId -EQ $RootProcessId
  if ($null -eq $live -or $live.CreationDate -ne $root.CreationDate -or $live.IdentityError -ne 0) {throw 'GPU observer root exited or identity became unavailable'}
  $products=@($snapshot | Where-Object {$_.SessionId -eq $root.SessionId -and $_.ExecutablePath -in $paths})
  $ids=@($products | ForEach-Object ProcessId)
  $filter=($ids | ForEach-Object {"Name LIKE 'pid_$($_)_%'"}) -join ' OR '
  # WQL LIKE uses '_' as a wildcard, so independently verify the full PID token.
  $memory=@(Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUProcessMemory -Filter $filter | Where-Object {$_.Name -match '^pid_(\d+)_' -and [int]$Matches[1] -in $ids} | Select-Object Name,DedicatedUsage,SharedUsage,TotalCommitted,Timestamp_PerfTime,Frequency_PerfTime)
  $engines=@(Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine -Filter $filter | Where-Object {$_.Name -match '^pid_(\d+)_' -and [int]$Matches[1] -in $ids} | Select-Object Name,UtilizationPercentage,Timestamp_PerfTime,Frequency_PerfTime)
  $record=[ordered]@{sample=$samples;utc=[datetime]::UtcNow.ToString('o');elapsed_s=$tick/1000;collection_ms=$clock.Elapsed.TotalMilliseconds-$tick;
   identities=@($products | Select-Object ProcessId,CreationDate,SessionId,ExecutablePath);memory=$memory;engines=$engines;
   counter_presence=$(if ($memory.Count -or $engines.Count) {'PRESENT'} else {'NO_COUNTERS_NOT_ZERO'})}
  ($record | ConvertTo-Json -Depth 5 -Compress) | Add-Content -LiteralPath (Join-Path $out 'gpu.jsonl')
  $samples++
  $remaining=1000-($clock.Elapsed.TotalMilliseconds-$tick)
  if ($remaining -gt 0) {Start-Sleep -Milliseconds ([int]$remaining)}
 }
} catch {$failure=$_.Exception.Message}
finally {
 Write-EchoJsonNew (Join-Path $out 'metadata.json') ([ordered]@{
  status=$(if ($null -eq $failure) {'MEASURED_UNVERIFIED'} else {'INCOMPLETE'});error=$failure;samples=$samples;duration_s=$clock.Elapsed.TotalSeconds;
  root_pid=$RootProcessId;root_creation_utc=$root.CreationDate;observer_sha256=(Get-FileHash $PSCommandPath).Hash;
  notes=@('GPU counters are separate from Private Bytes and must not be added to it.','Per-engine utilization is not one additive GPU utilization percentage.','Absent counters are not zero.','Exact-path/session candidates still require run ownership review.','Formatted counter refresh periods differ from observer sample times.');product_gate='NOT_EVALUATED'
 })
}
if ($null -ne $failure) {throw $failure}
