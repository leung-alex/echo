#requires -Version 7.0
<# Native tests for measurement tooling, not for Echo. Only test-owned PowerShell
   sleep processes are launched/stopped. The clipboard and real Echo are untouched. #>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$OutputDirectory)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Windows is required for collector integration tests.' }
. (Join-Path $PSScriptRoot 'Common.ps1')
$root = New-EchoEvidenceDirectory $OutputDirectory
$results = [Collections.Generic.List[object]]::new()
function Assert-That([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Assert-Throws([scriptblock]$Action, [string]$Fragment) {
    $caught = $null
    try { & $Action | Out-Null } catch { $caught = $_.Exception.Message }
    Assert-That ($null -ne $caught) 'Expected the operation to fail.'
    if ($Fragment) { Assert-That ($caught.Contains($Fragment)) "Unexpected rejection: $caught" }
}
function Test-Case([string]$Name, [scriptblock]$Action) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    try {
        & $Action | Out-Null
        $results.Add([ordered]@{ name=$Name; status='PASS'; duration_ms=$timer.Elapsed.TotalMilliseconds })
        Write-Host "PASS $Name"
    } catch {
        $results.Add([ordered]@{ name=$Name; status='FAIL'; duration_ms=$timer.Elapsed.TotalMilliseconds; error=$_.Exception.Message })
        Write-Host "FAIL $Name : $($_.Exception.Message)"
    } finally { $timer.Stop() }
}
function Start-OwnedFixture([int]$Seconds) {
    $hostExe = (Get-Process -Id $PID).Path
    return Start-Process -FilePath $hostExe -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-Command',"Start-Sleep -Seconds $Seconds") -PassThru -WindowStyle Hidden
}
function Stop-OwnedFixture([Diagnostics.Process]$Process) {
    try {
        if (-not $Process.HasExited) { $Process.Kill(); $Process.WaitForExit(5000) | Out-Null }
    } finally { $Process.Dispose() }
}

Test-Case 'all PowerShell files parse without syntax errors' {
    foreach ($file in Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1' -File) {
        $tokens=$null; $parseErrors=$null
        [System.Management.Automation.Language.Parser]::ParseFile($file.FullName,[ref]$tokens,[ref]$parseErrors) | Out-Null
        Assert-That ($parseErrors.Count -eq 0) ("$($file.Name): " + ($parseErrors | Out-String))
    }
}
Test-Case 'JSON evidence is UTF-8 and cannot be overwritten' {
    $path=Join-Path $root 'immutable.json'
    Write-EchoJsonNew $path @{ text='Echo 中文'; number=7 }
    $hash=(Get-FileHash -LiteralPath $path).Hash
    Assert-Throws { Write-EchoJsonNew $path @{ number=8 } } ''
    Assert-That ((Get-FileHash -LiteralPath $path).Hash -eq $hash) 'Original evidence changed.'
    Assert-That ((Get-Content -Raw -LiteralPath $path | ConvertFrom-Json).text -eq 'Echo 中文') 'UTF-8 roundtrip failed.'
}
Test-Case 'existing output directories are rejected' {
    $path=New-EchoEvidenceDirectory (Join-Path $root 'reserved')
    Assert-Throws { New-EchoEvidenceDirectory $path } 'new evidence directory'
}
Test-Case 'an existing reservation blocks a second collector' {
    $path=Join-Path $root 'claimed'
    [IO.File]::WriteAllText(($path+'.claim'),'test-owned marker')
    Assert-Throws { New-EchoEvidenceDirectory $path } ''
    Assert-That (-not (Test-Path -LiteralPath $path)) 'Reservation collision created output.'
}
Test-Case 'input/output containment is rejected but siblings are accepted' {
    Assert-Throws { Assert-EchoOutsideRoot $root (Join-Path $root 'inside.json') } 'outside'
    Assert-Throws { Assert-EchoOutsideRoot $root $root } 'outside'
    Assert-EchoOutsideRoot (Join-Path $root 'input') (Join-Path $root 'output.json')
}
Test-Case 'junction ancestors cannot redirect evidence writes' {
    $target=New-EchoEvidenceDirectory (Join-Path $root 'junction-target')
    $link=Join-Path $root 'junction-alias'
    New-Item -ItemType Junction -Path $link -Target $target | Out-Null
    try {
        Assert-Throws { Write-EchoJsonNew (Join-Path $link 'redirected.json') @{ invalid=$true } } 'Reparse-point'
        Assert-That (-not (Test-Path -LiteralPath (Join-Path $target 'redirected.json'))) 'Wrote through a junction.'
    } finally { [IO.Directory]::Delete($link) }
}
Test-Case 'baseline runner requires a dedicated test desktop opt-in' {
    $output=Join-Path $root 'must-not-exist'
    Assert-Throws { & (Join-Path $PSScriptRoot 'Invoke-BaselineGates.ps1') -RepositoryRoot $root -EvidenceRoot $output } 'DedicatedTestDesktop'
    Assert-That (-not (Test-Path -LiteralPath $output)) 'Unapproved run wrote evidence.'
}
Test-Case 'environment capture hashes a synthetic binary and does not invent metrics' {
    $binary=Join-Path $root 'synthetic.bin'
    [IO.File]::WriteAllBytes($binary,[byte[]](1,2,3,4))
    $output=Join-Path $root 'environment.json'
    & (Join-Path $PSScriptRoot 'Capture-Environment.ps1') -OutputFile $output -BinaryPath $binary -Variant 'TOOL_TEST'
    $data=Get-Content -Raw -LiteralPath $output | ConvertFrom-Json
    Assert-That ($data.schema -eq 'echo.environment.v1') 'Wrong environment schema.'
    Assert-That ($data.variant -eq 'TOOL_TEST') 'Lost synthetic test designation.'
    Assert-That ($data.binary.bytes -eq 4 -and $data.binary.sha256.Length -eq 64) 'Invalid binary inventory.'
    Assert-That ($data.manual_fields_required.Count -gt 0) 'Missing unmeasured-field declaration.'
    Assert-Throws { & (Join-Path $PSScriptRoot 'Capture-Environment.ps1') -OutputFile $output } ''
}
Test-Case 'live-process sampling records private memory and unknown ownership honestly' {
    $fixture=Start-OwnedFixture 60
    try {
        $output=Join-Path $root 'live-process'
        & (Join-Path $PSScriptRoot 'Measure-EchoProcessTree.ps1') -RootProcessId $fixture.Id -OutputDirectory $output -DurationSeconds 4 -IntervalMilliseconds 500
        $meta=Get-Content -Raw -LiteralPath (Join-Path $output 'metadata.json') | ConvertFrom-Json
        $rows=@(Import-Csv -LiteralPath (Join-Path $output 'aggregate.csv'))
        Assert-That ($meta.status -eq 'SAMPLED_UNVERIFIED') "Invalid live collection: $($meta.status)"
        Assert-That ($meta.sample_count -gt 0 -and $meta.valid_sample_count -eq $meta.sample_count) 'Missing valid samples.'
        Assert-That ($meta.process_set_verified -eq $false) 'Sampler cannot certify process ownership.'
        Assert-That ($meta.gpu -eq 'NOT_RUN' -and $meta.startup_ui_ready -eq 'NOT_MEASURED') 'Invented GPU/startup result.'
        Assert-That ($rows[0].cpu_one_core_percent -eq '') 'The first CPU interval must be missing, not zero.'
        Assert-That ([double]$rows[0].private_bytes -gt 0 -and [double]$rows[0].private_working_set_bytes -gt 0) 'No private-memory data.'
        Assert-That (-not $fixture.HasExited) 'Read-only sampler terminated the test process.'
        Assert-Throws { & (Join-Path $PSScriptRoot 'Measure-EchoProcessTree.ps1') -RootProcessId $fixture.Id -OutputDirectory $output -DurationSeconds 1 } 'new evidence directory'
    } finally { Stop-OwnedFixture $fixture }
}
Test-Case 'root exit produces incomplete evidence rather than a false low-memory result' {
    $fixture=Start-OwnedFixture 5
    try {
        $output=Join-Path $root 'exited-process'
        & (Join-Path $PSScriptRoot 'Measure-EchoProcessTree.ps1') -RootProcessId $fixture.Id -OutputDirectory $output -DurationSeconds 8 -IntervalMilliseconds 500
        $meta=Get-Content -Raw -LiteralPath (Join-Path $output 'metadata.json') | ConvertFrom-Json
        Assert-That ($meta.status -eq 'INCOMPLETE' -and $meta.root_exited -eq $true) 'Root exit was accepted as a complete run.'
    } finally { Stop-OwnedFixture $fixture }
}

$failures=@($results.ToArray() | Where-Object { $_.status -eq 'FAIL' })
Write-EchoJsonNew (Join-Path $root 'test-results.json') ([ordered]@{
    schema='echo.collector.tests.v1'; cases=@($results.ToArray()); failed=$failures.Count
    g0='NOT_RUN'; echo_started=$false; clipboard_touched=$false
    note='All sampled processes and binary data belong to this synthetic tooling test, not to Echo.'
})
if ($failures.Count -gt 0) { throw "$($failures.Count) collector test(s) failed; evidence: $root" }
Write-Output "Collector tests passed: $($results.Count). Echo product acceptance remains NOT_RUN."
