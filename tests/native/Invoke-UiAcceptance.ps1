[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Driver,
    [AllowEmptyString()][string]$Fixture,
    [Parameter(Mandatory)][ValidateSet('smoke','clipboard','quick-insert','ui')][string]$Scope,
    [Parameter(Mandatory)][string]$EvidenceRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:ECHO_WINDOWS_ACCEPTANCE -ne '1') { throw 'Explicit native acceptance authorization is required.' }
$mainTitle = 'Echo Recall'
$favoritesTitle = 'Echo Favorites'
$checks = [Collections.Generic.List[object]]::new()
$echoProcess = $null
$targetProcess = $null
$targetRoot = $null
$target = $null
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
function Click([string]$Name,[string]$Title=$mainTitle) { D invoke $Title @($Name) | Out-Null }
function Select-Row([string]$Name,[string]$Title=$mainTitle) { D select $Title @($Name) | Out-Null }
function Set-Value([string]$Name,[string]$Value,[string]$Title=$mainTitle) { D value $Title @($Name,$Value) | Out-Null }
function Query([string]$Value,[string]$Title=$mainTitle) { Set-Value 'Search clipboard history' $Value $Title; Wait-Until { (D read $Title @('Search clipboard history')) -eq $Value } "query '$Value'" | Out-Null }
function Shot([string]$Name,[string]$Title=$mainTitle) { D capture $Title @((Join-Path $EvidenceRoot "$Name.png")) | Out-Null }
function Start-OwnedEcho([string[]]$Arguments=@()) {
    $script:launchCount++; $stdout = Join-Path $EvidenceRoot ('echo-'+$launchCount+'.stdout.log'); $stderr = Join-Path $EvidenceRoot ('echo-'+$launchCount+'.stderr.log')
    $launch = @{FilePath=$Executable;PassThru=$true;RedirectStandardOutput=$stdout;RedirectStandardError=$stderr}; if($Arguments.Count){$launch.ArgumentList=$Arguments}; $script:echoProcess=Start-Process @launch
    [Environment]::SetEnvironmentVariable('ECHO_ACCEPTANCE_PID',[string]$script:echoProcess.Id,'Process')
}
function Start-ScopedProcess([string[]]$Arguments) {
    $process = Start-Process -FilePath $Executable -ArgumentList $Arguments -PassThru
    if (!$process.WaitForExit(10000)) { throw "Scoped Echo handoff timed out: $($Arguments -join ' ')" }
    if ($process.ExitCode -ne 0) { throw "Scoped Echo handoff failed with $($process.ExitCode): $($Arguments -join ' ')" }
}
function Fixture([string[]]$Arguments) {
    if (!$Fixture) { throw 'A prebuilt ECHO_ACCEPTANCE_FIXTURE_EXE is required.' }
    $output = & $Fixture @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Fixture failed: $($Arguments -join ' ')`n$($output -join "`n")" }
    return ($output -join "`n").Trim()
}
function Activation([string]$Action,[hashtable]$Payload=@{}) {
    $envelope = [ordered]@{version=1;request_id=[guid]::NewGuid().ToString();action=$Action;origin=$null;payload=$Payload}
    $bytes = [Text.Encoding]::UTF8.GetBytes(($envelope | ConvertTo-Json -Compress -Depth 8))
    return [Convert]::ToBase64String($bytes).TrimEnd('=').Replace('+','-').Replace('/','_')
}
function Start-Target {
    $script:targetRoot = Join-Path $EvidenceRoot ('target-' + [guid]::NewGuid().ToString('N')); New-Item -ItemType Directory $script:targetRoot | Out-Null
    $runId=[guid]::NewGuid().ToString(); $title="Echo target fixture $runId"
    $paths=@{ready=(Join-Path $targetRoot 'ready.json');command=(Join-Path $targetRoot 'command.json');response=(Join-Path $targetRoot 'response.json');primary=(Join-Path $targetRoot 'primary.txt');secondary=(Join-Path $targetRoot 'secondary.txt');password=(Join-Path $targetRoot 'password.txt');readonly=(Join-Path $targetRoot 'readonly.txt');unknown=(Join-Path $targetRoot 'unknown.txt')}
    $args=@('target','--run-id',$runId,'--title',$title,'--ready',$paths.ready,'--command',$paths.command,'--response',$paths.response,'--primary',$paths.primary,'--secondary',$paths.secondary,'--password',$paths.password,'--readonly',$paths.readonly,'--unknown',$paths.unknown)
    $script:targetProcess=Start-Process -FilePath $Fixture -ArgumentList $args -PassThru
    Wait-Until { Test-Path -LiteralPath $paths.ready } 'owned target readiness' 30000 | Out-Null
    $ready=Get-Content -Raw $paths.ready|ConvertFrom-Json
    if ($ready.run_id -ne $runId -or $ready.process_id -ne $targetProcess.Id -or $ready.title -ne $title) { throw 'Target fixture identity does not belong to this run.' }
    return [pscustomobject]@{runId=$runId;title=$title;paths=$paths;next=0}
}
function Target-Command($Target,[string]$Command,[string]$Payload='') {
    $Target.next++; $requestId="$($Target.runId)-$PID-$($Target.next)"; Remove-Item -LiteralPath $Target.paths.response -Force -ErrorAction SilentlyContinue
    $temporary=$Target.paths.command+'.tmp'; [ordered]@{run_id=$Target.runId;request_id=$requestId;command=$Command;payload=$Payload}|ConvertTo-Json -Compress|Set-Content -LiteralPath $temporary -Encoding utf8NoBOM
    Move-Item -LiteralPath $temporary -Destination $Target.paths.command
    $responseWait=[Diagnostics.Stopwatch]::StartNew()
    while(!(Test-Path -LiteralPath $Target.paths.response)) {
        if($targetProcess.HasExited -and $Command -eq 'shutdown'){return ''}
        if($targetProcess.HasExited){throw "Owned target exited during $Command"}
        if($responseWait.ElapsedMilliseconds -gt 10000){throw "Owned target response timeout: $Command"}
        Start-Sleep -Milliseconds 20
    }
    $response=Get-Content -Raw $Target.paths.response|ConvertFrom-Json
    if ($response.request_id -ne $requestId) { throw 'Target fixture response identity changed.' }
    $error=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($response.error)); if ($error) { throw "Target command $Command failed: $error" }
    return [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($response.value))
}

try {
    Start-OwnedEcho
    Check 'semantic-startup-one-window' {
        Wait-Until { D ready } 'owned main semantic readiness' 20000 | Out-Null
        if ((D window-count) -ne 1) { throw 'Expected one owned native Echo window.' }
        if($Scope -eq 'ui'){Shot 'startup-main-light'; Shot 'startup-favorites-light' $favoritesTitle}
        'one owned window exposes native fixture content'
    }

    if ($Scope -eq 'smoke') {
        Check 'smoke-search' { Query '0013'; Wait-Text 'echo-perf-text-0013'; 'exact fixture result observed' }
        Check 'close-to-hide' { D close $mainTitle | Out-Null; Wait-Hidden; if ($echoProcess.HasExited) { throw 'Closing terminated the resident.' }; 'owned WM_CLOSE hides the window and preserves the resident' }
        Start-ScopedProcess @('--history'); Wait-Until { D exists } 'activation reopen' | Out-Null
        Check 'graceful-exit' { Start-ScopedProcess @('--quit'); if (!$echoProcess.WaitForExit(10000) -or $echoProcess.ExitCode -ne 0) { throw 'Owned resident did not exit cleanly.' }; "exit=$($echoProcess.ExitCode)" }
        Not-Run 'clipboard-roundtrip' 'Smoke scope intentionally does not touch the clipboard.'
        return
    }

    if ($Scope -eq 'clipboard') {
        Check 'copy-roundtrip' { Query '0013'; Wait-Text 'echo-perf-text-0013'; Select-Row 'echo-perf-text-0013 — Reusable content, available when you need it.'; Click 'Copy item'; $actual=Fixture @('clipboard','--operation','read-text'); if (!$actual.Contains('echo-perf-text-0013')) { throw "Clipboard differed: $actual" }; $actual }
    }

    if ($Scope -eq 'ui') {
        Check 'search' { Query '0013'; Wait-Text '1 captures loaded'; Wait-Text 'echo-perf-text-0013'; Shot 'search'; 'one exact result' }
        Check 'history-favorite-create-edit-delete' {
            Select-Row 'echo-perf-text-0013 — Reusable content, available when you need it.'; Click 'Add to Favorites'; Wait-Text 'Added to Favorites'
            Click 'Favorites'; Click 'Create favorite'; Set-Value 'Favorite name' 'Native acceptance favorite'; Set-Value 'Favorite content' 'echo-native-test-content-20260906'; Set-Value 'Favorite tags' 'native,test'; Click 'Choose icon'; Wait-Text 'Choose an icon'; Set-Value 'Search favorite icons' 'mAiL'; Wait-Text 'Icon Mail'; Shot 'icon-picker'; Click 'Icon Mail'; if((D read $mainTitle @('Favorite icon')) -ne 'Mail'){throw 'Picker did not preserve Mail key'}; Shot 'favorite-create'; Click 'Save favorite'; Wait-Text 'Favorite created'
            Query 'echo-native-test-content-20260906'; Wait-Text 'Native acceptance favorite'; Select-Row 'Native acceptance favorite'; Click 'Edit favorite'; if((D read $mainTitle @('Favorite icon')) -ne 'Mail'){throw 'Saved favorite icon did not persist'}; Set-Value 'Favorite name' 'Edited native favorite'; Set-Value 'Favorite content' 'echo-native-updated-content-20260906'; Shot 'favorite-edit'; Click 'Save favorite'; Wait-Text 'Favorite updated'; Query 'echo-native-updated-content-20260906'; Wait-Text 'Edited native favorite'; Click 'Delete item'; Wait-Text 'No matches'
            'create/edit/delete observed through live UI; persistence checked after restart below'
        }
        Check 'batch-and-clear-cancel' { Click 'History'; Query ''; Click 'Select'; Click 'All loaded'; Wait-Text 'selected'; Click 'Cancel'; Click 'Clear all'; Wait-Text 'Clear clipboard history?'; Shot 'clear-confirmation'; Click 'Keep history'; 'batch selection and non-destructive clear cancellation observed' }
        Check 'settings-invalid-valid-theme-about' {
            Click 'Settings'; Set-Value 'Maximum history entries' '0'; Click 'Save settings'; Wait-Text 'greater than zero'
            Set-Value 'Maximum history entries' '5001'; Set-Value 'Total storage MiB' '512'; Set-Value 'Maximum item MiB' '32'; Click 'Save settings'; Wait-Text 'Settings saved'; Shot 'settings-light'
            D theme $mainTitle @('dark') | Out-Null; Click 'Save settings'; Wait-Text 'Settings saved'; Shot 'settings-dark'
            Click 'Back'; Shot 'history-dark'; Click 'Settings'
            D theme $mainTitle @('light') | Out-Null; Click 'Save settings'; Wait-Text 'Settings saved'
            Click 'About Echo'; Wait-Text 'Rust + Slint'; Shot 'about-slint'; Click 'Back to settings'
            'invalid rejected; valid saved; Slint about visible'
        }
        Check 'image-thumbnail-hover' { Click 'Back'; Click 'History'; Query 'Echo fixture 0000'; Wait-Text 'Fixture image 0000'; D hover $mainTitle @('Fixture image 0000 · 1280 × 720')|Out-Null; Shot 'image-hover'; 'fixture thumbnail rendered and hovered' }
        Check 'favorites-only-close' { D close $favoritesTitle|Out-Null; Wait-Hidden $favoritesTitle; if (!(D exists)) { throw 'Main hid with Favorites.' }; 'Favorites hidden; main remains visible' }
        Check 'second-instance-handoff-and-malformed-envelope' { Click 'Hide Echo'; Wait-Hidden; Start-ScopedProcess @('--history'); Wait-Until { D exists } 'history handoff reopen'|Out-Null; $bad=Start-Process -FilePath $Executable -ArgumentList @('--echo-activate','not-base64url') -PassThru; if(!$bad.WaitForExit(5000) -or $bad.ExitCode -eq 0){throw 'Malformed envelope was accepted'}; if (!(D exists)) { throw 'Malformed envelope disturbed resident.' }; 'valid handoff reopened; malformed envelope exited without replacing resident' }
        Check 'activation-replay' { $token=Activation 'echo.open'; Start-ScopedProcess @('--echo-activate',$token); Start-ScopedProcess @('--echo-activate',$token); if (!(D exists)) { throw 'Replay disturbed resident.' }; 'same request delivered twice without spawning a second resident' }
        Check 'settings-and-favorite-persist-after-restart' { Start-ScopedProcess @('--quit'); if (!$echoProcess.WaitForExit(10000)) { throw 'First resident did not exit.' }; Start-OwnedEcho; Wait-Until { D ready } 'restart readiness' 20000|Out-Null; Click 'Settings'; if((D read $mainTitle @('Maximum history entries')) -ne '5001'){throw 'Saved setting did not persist'}; Click 'Back'; Click 'Favorites'; Query 'echo-native-updated-content-20260906'; Wait-Text 'No matches'; 'settings persisted; deleted favorite stayed deleted in same database identity' }
    }

    if ($Scope -eq 'quick-insert') {
        $target=Start-Target
        Check 'safe-paste-owned-target' {
            Target-Command $target 'focus-primary'|Out-Null; Target-Command $target 'allow-foreground' ([string]$echoProcess.Id)|Out-Null
            Start-ScopedProcess @('--echo-activate',(Activation 'echo.quick_insert' @{query='echo-perf-text-0013'})); Wait-Text 'echo-perf-text-0013'; Select-Row 'echo-perf-text-0013 — Reusable content, available when you need it.'; Click 'Insert item'
            Wait-Until { (Target-Command $target 'read-primary').Contains('echo-perf-text-0013') } 'actual delivery to owned text control' | Out-Null; $actual=Target-Command $target 'read-primary'; $actual
        }
        Check 'paste-failure-restores-clipboard' {
            $sentinel='Echo failure restore sentinel '+[guid]::NewGuid(); Fixture @('clipboard','--operation','copy-text','--value',$sentinel)|Out-Null
            Target-Command $target 'focus-readonly'|Out-Null; Target-Command $target 'allow-foreground' ([string]$echoProcess.Id)|Out-Null
            Start-ScopedProcess @('--echo-activate',(Activation 'echo.quick_insert' @{query='echo-perf-text-0013'})); Wait-Text 'echo-perf-text-0013'; Select-Row 'echo-perf-text-0013 — Reusable content, available when you need it.'; Click 'Insert item'; Wait-Text 'no safe paste target'
            $actual=Fixture @('clipboard','--operation','read-text'); if ($actual -ne $sentinel) { throw "Clipboard was not restored. actual=$actual" }; $actual
        }
    }

    Check 'graceful-exit' { Start-ScopedProcess @('--quit'); if (!$echoProcess.WaitForExit(10000) -or $echoProcess.ExitCode -ne 0) { throw 'Owned resident did not exit cleanly.' }; "exit=$($echoProcess.ExitCode)" }
}
finally {
    if ($targetProcess -and !$targetProcess.HasExited) {
        try { if ($target) { Target-Command $target 'shutdown'|Out-Null; if(!$targetProcess.WaitForExit(10000)){throw 'Owned target did not terminate'}; Add-Check 'target-cleanup' 'PASS' 'Owned target exited' 'owned target graceful shutdown' } } catch { Add-Check 'target-cleanup' 'FAIL' $_.Exception.Message 'owned target graceful shutdown' $_.ToString() }
    }
    if ($echoProcess -and !$echoProcess.HasExited) {
        try { Start-ScopedProcess @('--quit'); if (!$echoProcess.WaitForExit(10000)) { throw 'Owned Echo did not quit.' } } catch { Add-Check 'echo-cleanup' 'FAIL' $_.Exception.Message 'data-scoped graceful quit' $_.ToString() }
    }
    if (!(Test-Path -LiteralPath (Join-Path $EvidenceRoot 'checks.json'))) { $checks | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'checks.json') -Encoding utf8NoBOM }
}
