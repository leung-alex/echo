[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Driver,
    [Parameter(Mandatory)][ValidateSet('smoke')][string]$Scope,
    [Parameter(Mandatory)][string]$EvidenceRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$mainTitle = 'Echo Recall'
$checks = [Collections.Generic.List[object]]::new()
$echoProcess = $null
$launchCount = 0

function Add-Check([string]$Name,[string]$Status,[string]$Actual,[string]$Expected,[string]$Error='') {
    $checks.Add([ordered]@{name=$Name;status=$Status;actual=$Actual;expected=$Expected;error=$Error})
    $checks | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'checks.json') -Encoding utf8NoBOM
}
function Save-Diagnostics([string]$Name) {
    foreach ($title in @($mainTitle)) {
        try { D dump $title | Set-Content -LiteralPath (Join-Path $EvidenceRoot ("failure-{0}-{1}.uia.txt" -f $Name,($title -replace ' ','-'))) -Encoding utf8NoBOM } catch {}
        # Screenshots are restricted to the explicit native-test suite; retain UIA diagnostics here.
    }
}
function Check([string]$Name,[scriptblock]$Body,[string]$Expected='assertions in check body') {
    try { $actual = & $Body; Add-Check $Name 'PASS' ([string]$actual) $Expected; Write-Host "PASS $Name" }
    catch { Save-Diagnostics $Name; Add-Check $Name 'FAIL' $_.Exception.Message $Expected $_.ToString(); throw }
}
function Not-Run([string]$Name,[string]$Reason) { Add-Check $Name 'NOT_RUN' $Reason 'explicit execution required' }
function D([string]$Operation,[string]$Title=$mainTitle,[string[]]$Arguments=@()) {
    if (!$echoProcess -or $echoProcess.HasExited) { throw 'Owned Echo process is unavailable.' }
    $info=[Diagnostics.ProcessStartInfo]::new($Driver)
    $info.UseShellExecute=$false; $info.CreateNoWindow=$true
    $info.RedirectStandardOutput=$true; $info.RedirectStandardError=$true
    foreach($argument in @($Operation,[string]$echoProcess.Id,$Title)+$Arguments){$info.ArgumentList.Add($argument)}
    $child=[Diagnostics.Process]::Start($info)
    try {
        $stdout=$child.StandardOutput.ReadToEndAsync();$stderr=$child.StandardError.ReadToEndAsync()
        if(!$child.WaitForExit(15000)){$child.Kill();$child.WaitForExit(2000)|Out-Null;throw "Owned UIA driver timed out: $Operation $Title"}
        $output=$stdout.GetAwaiter().GetResult();$errorText=$stderr.GetAwaiter().GetResult()
        if($child.ExitCode -ne 0){throw "Driver failed: $Operation $Title $($Arguments -join ', ')`n$errorText"}
        $parsed=$output.TrimStart([char]0xfeff)|ConvertFrom-Json
        if($parsed.status -ne 'PASS'){throw "Driver did not pass: $Operation"}
        return $parsed.value
    } finally {$child.Dispose()}
}
function Wait-Until([scriptblock]$Probe,[string]$Description,[int]$TimeoutMs=10000) {
    $watch = [Diagnostics.Stopwatch]::StartNew(); $last = $null
    while ($watch.ElapsedMilliseconds -lt $TimeoutMs) {
        if ($echoProcess -and $echoProcess.HasExited) { throw "Echo exited while waiting for $Description (exit $($echoProcess.ExitCode))." }
        try { $last = & $Probe; if ($last) { return $last } } catch { $last = $_.Exception.Message }
        Start-Sleep -Milliseconds 20
    }
    throw "Timed out waiting for $Description. Last observation: $last"
}
function Wait-Text([string]$Text,[string]$Title=$mainTitle,[int]$TimeoutMs=10000) {
    Wait-Until { (D dump $Title).Contains($Text) } "text '$Text' in $Title" $TimeoutMs | Out-Null
}
function Wait-Hidden([string]$Title=$mainTitle) { Wait-Until { !(D exists $Title) } "$Title to hide" | Out-Null }
function Start-OwnedEcho([string[]]$Arguments=@()) {
    $script:launchCount++; $stdout = Join-Path $EvidenceRoot ('echo-'+$launchCount+'.stdout.log'); $stderr = Join-Path $EvidenceRoot ('echo-'+$launchCount+'.stderr.log')
    $launch = @{FilePath=$Executable;PassThru=$true;RedirectStandardOutput=$stdout;RedirectStandardError=$stderr;WindowStyle="Hidden"}; if($Arguments.Count){$launch.ArgumentList=$Arguments}; $script:echoProcess=Start-Process @launch
    [Environment]::SetEnvironmentVariable('ECHO_ACCEPTANCE_PID',[string]$script:echoProcess.Id,'Process')
}
function Start-ScopedProcess([string[]]$Arguments) {
    $process = Start-Process -FilePath $Executable -ArgumentList $Arguments -WindowStyle Hidden -PassThru
    try {
        if (!$process.WaitForExit(10000)) {
            $process.Kill(); $process.WaitForExit(5000) | Out-Null
            throw "Scoped Echo handoff timed out; owned secondary was terminated: $($Arguments -join ' ')"
        }
        if ($process.ExitCode -ne 0) { throw "Scoped Echo handoff failed with $($process.ExitCode): $($Arguments -join ' ')" }
    } finally { $process.Dispose() }

}

try {
    Start-OwnedEcho
    Check 'semantic-startup-one-window' {
        Wait-Until { D ready } 'owned main semantic readiness' 20000 | Out-Null
        if ((D window-count) -ne 1) { throw 'Expected one owned native Echo window.' }
        'one owned window exposes native fixture content'
    }

    if ($Scope -eq 'smoke') {
        Check 'smoke-manual-history' { Wait-Text 'History space'; if ((D dump).Contains('ControlType.Edit | Search clipboard history')) { throw 'Removed local search field is exposed.' }; 'native history is ready without a local search field' }
        Check 'close-to-hide' { D close $mainTitle | Out-Null; Wait-Hidden; if ($echoProcess.HasExited) { throw 'Closing terminated the resident.' }; 'owned WM_CLOSE hides the window and preserves the resident' }
        Start-ScopedProcess @('--history'); Wait-Until { D exists } 'activation reopen' | Out-Null
        Check 'graceful-exit' { Start-ScopedProcess @('--quit'); if (!$echoProcess.WaitForExit(10000) -or $echoProcess.ExitCode -ne 0) { throw 'Owned resident did not exit cleanly.' }; "exit=$($echoProcess.ExitCode)" }
        Not-Run 'clipboard-roundtrip' 'Smoke scope intentionally does not touch the clipboard.'
        return
    }


}
finally {
    if ($echoProcess -and !$echoProcess.HasExited) {
        try { Start-ScopedProcess @('--quit'); if (!$echoProcess.WaitForExit(10000)) { throw 'Owned Echo did not quit.' } } catch { Add-Check 'echo-cleanup' 'FAIL' $_.Exception.Message 'data-scoped graceful quit' $_.ToString() }
    }
    if (!(Test-Path -LiteralPath (Join-Path $EvidenceRoot 'checks.json'))) { $checks | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'checks.json') -Encoding utf8NoBOM }
}
