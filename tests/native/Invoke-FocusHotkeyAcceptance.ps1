[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Driver,
    [Parameter(Mandatory)][string]$FocusDriver,
    [Parameter(Mandatory)][string]$Fixture,
    [Parameter(Mandatory)][string]$Template,
    [Parameter(Mandatory)][string]$EvidenceRoot,
    [ValidateSet('baseline','performance','feature')][string]$Scope='feature',
    [ValidateSet('software')][string]$Renderer='software',
    [switch]$NativeTest,
    [switch]$BrowserCompatibility
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if($env:ECHO_WINDOWS_ACCEPTANCE -ne '1'){throw 'Explicit native acceptance authorization required.'}
foreach($name in @('Root','Executable','Driver','FocusDriver','Fixture','Template','EvidenceRoot')) {Set-Variable $name ([IO.Path]::GetFullPath((Get-Variable $name -ValueOnly)))}
$marker=Get-Content (Join-Path $Template 'synthetic-fixture.json') -Raw|ConvertFrom-Json
if(!$marker.synthetic -or $marker.capture_enabled){throw 'Only capture-disabled synthetic fixtures are allowed.'}
if(Test-Path $EvidenceRoot){throw 'Evidence root must be new.'}
New-Item -ItemType Directory $EvidenceRoot|Out-Null
Copy-Item $Template (Join-Path $EvidenceRoot 'data') -Recurse
$old=@{};foreach($name in @('ECHO_DATA_DIR','ECHO_ACCEPTANCE_RUN_ROOT','ECHO_ACCEPTANCE_PID','ECHO_RENDERER','ECHO_NATIVE_TEST_ROOT','ECHO_DISABLE_GLOBAL_HOTKEY')){$old[$name]=[Environment]::GetEnvironmentVariable($name,'Process')}
$mainTitle='Echo Recall';$favoritesTitle='Echo Favorites'
$checks=[Collections.Generic.List[object]]::new();$ownedRecords=[Collections.Generic.List[object]]::new()
$browserProcess=$null;$browserTitle=$null;$echoProcess=$null;$targetProcess=$null;$targetRoot=$null;$target=$null;$blocker=$null;$held=$null;$launchCount=0;$success=$false

function Add-Check([string]$Name,[string]$Status,[string]$Actual,[string]$Expected,[string]$Error='') {
    $checks.Add([ordered]@{name=$Name;status=$Status;actual=$Actual;expected=$Expected;error=$Error})
    $checks | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'checks.json') -Encoding utf8NoBOM
}
function Save-Diagnostics([string]$Name) {
    foreach ($title in @($mainTitle)) {
        try { D dump $title | Set-Content -LiteralPath (Join-Path $EvidenceRoot ("failure-{0}-{1}.uia.txt" -f $Name,($title -replace ' ','-'))) -Encoding utf8NoBOM } catch {}
        if($NativeTest){try{D metrics|ConvertTo-Json -Depth 20|Set-Content (Join-Path $EvidenceRoot ("failure-$Name-metrics.json")) -Encoding utf8NoBOM;Shot ("failure-$Name")}catch{}}
        if(Get-Variable lastTargetGeometry -Scope Script -ErrorAction SilentlyContinue){$script:lastTargetGeometry|ConvertTo-Json -Depth 8|Set-Content (Join-Path $EvidenceRoot ("failure-$Name-target.json")) -Encoding utf8NoBOM}
    }
}
function Check([string]$Name,[scriptblock]$Body,[string]$Expected='assertions in check body') {
    try { $actual = & $Body; Add-Check $Name 'PASS' ([string]$actual) $Expected; Write-Host "PASS $Name" }
    catch { Save-Diagnostics $Name; Add-Check $Name 'FAIL' $_.Exception.Message $Expected $_.ToString(); Write-Host "FAIL $Name : $($_.Exception.Message)"; if($Name -notin @('actual-alt-v-caret-anchored-popup','edge-positions-fit-and-flip-without-covering-caret','quick-insert-focus-loss-dismisses-without-pasting','rebound-shortcut-anchor')){throw} }
}
function Not-Run([string]$Name,[string]$Reason) { Add-Check $Name 'NOT_RUN' $Reason 'explicit execution required' }
function D([string]$Operation,[string]$Title=$mainTitle,[string[]]$Arguments=@()) {
    if (!$echoProcess -or $echoProcess.HasExited) { throw 'Owned Echo process is unavailable.' }
    $info=[Diagnostics.ProcessStartInfo]::new($Driver)
    $info.UseShellExecute=$false; $info.CreateNoWindow=$true
    $info.RedirectStandardOutput=$true; $info.RedirectStandardError=$true; $info.StandardOutputEncoding=[Text.Encoding]::UTF8; $info.StandardErrorEncoding=[Text.Encoding]::UTF8
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
    Record-Owned $script:echoProcess $mainTitle
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
    Record-Owned $script:targetProcess $title
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


function Record-Owned($Process,[string]$Title) {
    $clock=[Diagnostics.Stopwatch]::StartNew();$executablePath=$null
    do {$Process.Refresh();if($Process.HasExited){throw 'Owned process exited before identity capture'};try{$executablePath=$Process.MainModule.FileName}catch{};if(!$executablePath){Start-Sleep -Milliseconds 10}}while(!$executablePath -and $clock.ElapsedMilliseconds -lt 5000)
    if(!$executablePath){throw 'Cannot verify the owned executable path'}
    $ownedRecords.Add([ordered]@{pid=$Process.Id;title=$Title;executable=$executablePath;started_utc=$Process.StartTime.ToUniversalTime().ToString('o')})
    ConvertTo-Json -InputObject @($ownedRecords.ToArray()) -Depth 5|Set-Content (Join-Path $EvidenceRoot 'owned-processes.json') -Encoding utf8NoBOM
}
function F([string]$Operation,[string[]]$Arguments=@()) {
    $info=[Diagnostics.ProcessStartInfo]::new($FocusDriver)
    $info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true;$info.StandardOutputEncoding=[Text.Encoding]::UTF8;$info.StandardErrorEncoding=[Text.Encoding]::UTF8
    foreach($arg in @($Operation,$EvidenceRoot)+$Arguments){$info.ArgumentList.Add($arg)}
    $child=[Diagnostics.Process]::Start($info)
    try {
        $stdout=$child.StandardOutput.ReadToEndAsync();$stderr=$child.StandardError.ReadToEndAsync()
        if($Operation -eq 'focus-edit-permitted'){if(!$target -or $targetProcess.HasExited){throw 'No owned foreground grant provider is available'};Target-Command $target 'allow-foreground' ([string]$child.Id)|Out-Null;[IO.File]::WriteAllText((Join-Path $EvidenceRoot ("foreground-permit-"+$child.Id)),'owned process grant')}
        if(!$child.WaitForExit($(if($Operation -eq 'cycles'){120000}else{20000}))){$child.Kill();throw "Integration driver timed out: $Operation"}
        if($child.ExitCode -ne 0){throw $stderr.Result}
        return ($stdout.Result.TrimStart([char]0xfeff)|ConvertFrom-Json).value
    }finally{$child.Dispose()}
}
function Open-KeyboardSettings {
    Start-ScopedProcess @('--settings');Wait-Text 'Settings'
    if((D dump).Contains('ControlType.Button | Keyboard & insertion |')){Click 'Keyboard & insertion'}
    else{D combo $mainTitle @('Settings category','2')|Out-Null}
    Wait-Text 'Global quick insert shortcut'
}
function Assert-Binding([string]$Chord,[bool]$Available) {
    $clock=[Diagnostics.Stopwatch]::StartNew();$last=$null
    do {$last=F 'probe' @($Chord);if($last.available -eq $Available){if(!$Available -and $last.error -ne 1409){throw "Shortcut probe failed with unexpected error $($last.error): $Chord"};return};Start-Sleep -Milliseconds 20}while($clock.ElapsedMilliseconds -lt 5000)
    throw "Binding $Chord expected availability $Available; actual $($last|ConvertTo-Json -Compress)"
}
function Open-FromTarget([string]$Chord='Alt+V',[string]$Control='primary') {
    if(D exists){D close|Out-Null;Wait-Hidden}
    Target-Command $target "focus-$Control"|Out-Null
    $script:lastTargetGeometry=F 'geometry' @([string]$targetProcess.Id,$target.title)
    F 'hotkey' @([string]$targetProcess.Id,$target.title,$Chord)|Out-Null
    Wait-Until {D exists} 'global hotkey popup' 5000|Out-Null
    Wait-Text 'History space'
}
function Assert-Anchored([string]$Name) {
    $card=F 'card' @([string]$echoProcess.Id,$mainTitle,'History space')
    $work=$lastTargetGeometry.work;$caret=$lastTargetGeometry.caret
    $scale=if($lastTargetGeometry.monitor_dpi -gt 0){$lastTargetGeometry.monitor_dpi/96.0}else{$lastTargetGeometry.dpi/96.0}
    if(!$caret -or $caret[3] -le $caret[1]){throw 'Owned source did not expose a measurable native caret.'}
    if($card[2]-$card[0] -lt 100 -or $card[3]-$card[1] -lt 100){throw 'Visible card bounds are empty.'}
    if($card[0] -lt $work[0]-3 -or $card[1] -lt $work[1]-3 -or $card[2] -gt $work[2]+3 -or $card[3] -gt $work[3]+3){throw 'Visible front card leaves the target work area.'}
    if(!($card[1] -ge $caret[3]-3 -or $card[3] -le $caret[1]+3)){throw "Card obscures caret line: card=$card caret=$caret"}
    $gap=[Math]::Min([Math]::Abs($card[1]-$caret[3]),[Math]::Abs($caret[1]-$card[3]))
    if($gap -gt 40*$scale){throw "Card is not near the caret: vertical gap=$gap"}
    $horizontalGap=[Math]::Max(0,[Math]::Max($card[0]-$caret[2],$caret[0]-$card[2]))
    if($horizontalGap -gt 40*$scale){throw "Visible card is too far horizontally from caret: $horizontalGap pixels"}
    $proof=[ordered]@{horizontal_gap_pixels=$horizontalGap;target=$lastTargetGeometry;card=$card;vertical_gap_pixels=$gap;source='independent Win32 caret and UIA card bounds'}
    if($NativeTest){$proof.metrics=D metrics;Shot $Name}
    $proof|ConvertTo-Json -Depth 15|Set-Content (Join-Path $EvidenceRoot "$Name.json") -Encoding utf8NoBOM
}

try {
    $env:ECHO_DATA_DIR=Join-Path $EvidenceRoot 'data'
    $env:ECHO_ACCEPTANCE_RUN_ROOT=$EvidenceRoot
    $env:ECHO_RENDERER=$Renderer
    $env:ECHO_DISABLE_GLOBAL_HOTKEY=if($Scope -eq 'baseline'){'1'}else{'0'}
    $env:ECHO_NATIVE_TEST_ROOT=if($NativeTest){$EvidenceRoot}else{$null}
    [ordered]@{schema='echo.focus-hotkey.acceptance.v1';source_head=(& git -C $Root rev-parse HEAD);machine=$env:COMPUTERNAME;scope=$Scope;renderer=$Renderer;native_test=[bool]$NativeTest;executable=$Executable;sha256=(Get-FileHash $Executable -Algorithm SHA256).Hash;started_utc=[DateTime]::UtcNow.ToString('o')}|ConvertTo-Json|Set-Content (Join-Path $EvidenceRoot 'identity.json') -Encoding utf8NoBOM
    $start=[Diagnostics.Stopwatch]::StartNew()
    Start-OwnedEcho @('--history')
    Check 'semantic-startup-one-window' {
        Wait-Until {D ready} 'main semantic readiness' 30000|Out-Null
        if((D window-count) -ne 1){throw 'Expected one native Echo window.'}
        [ordered]@{elapsed_ms=$start.Elapsed.TotalMilliseconds;note='Process launch to UIA semantic content readiness; not first display presentation.'}|ConvertTo-Json|Set-Content (Join-Path $EvidenceRoot 'startup.json') -Encoding utf8NoBOM
        'one native window, synthetic content loaded'
    }
    if($Scope -eq 'feature') {
        . (Join-Path $Root 'tests/native/FocusHotkeyScenarios.ps1')
        if($BrowserCompatibility){. (Join-Path $Root 'tests/native/FocusBrowserScenarios.ps1')}else{Not-Run 'chrome-compatibility' 'Optional external-browser compatibility was not requested; the native gate has no browser dependency.'}
    }
    Check 'resident-reopen-and-hidden-resource-sample' {
        if(!(D exists)){Start-ScopedProcess @('--history');Wait-Until {D ready} 'manager ready'|Out-Null}
        $measurement=F 'cycles' @([string]$echoProcess.Id,$mainTitle,'12')
        $measurement|ConvertTo-Json -Depth 8|Set-Content (Join-Path $EvidenceRoot 'reopen-and-resources.json') -Encoding utf8NoBOM
        $all=Get-CimInstance Win32_Process -Property ProcessId,ParentProcessId
        $ids=[Collections.Generic.HashSet[int]]::new();$null=$ids.Add($echoProcess.Id)
        do{$added=$false;foreach($entry in $all){if($ids.Contains([int]$entry.ParentProcessId) -and $ids.Add([int]$entry.ProcessId)){$added=$true}}}while($added)
        $tree=@(foreach($id in $ids){try{$owned=[Diagnostics.Process]::GetProcessById($id);[ordered]@{pid=$id;name=$owned.ProcessName;private_bytes=$owned.PrivateMemorySize64;working_set_bytes=$owned.WorkingSet64}}catch{}})
        [ordered]@{root_pid=$echoProcess.Id;processes=$tree;note='Root and actual child process tree only. Private committed bytes and total working set are distinct; shared OS/driver services excluded.'}|ConvertTo-Json -Depth 6|Set-Content (Join-Path $EvidenceRoot 'process-tree.json') -Encoding utf8NoBOM
        "12 measured manager reopen cycles, 2 warmups; hidden process resources recorded"
    }
    Check 'graceful-exit' {
        Start-ScopedProcess @('--quit')
        if(!$echoProcess.WaitForExit(10000) -or $echoProcess.ExitCode -ne 0){throw 'Echo did not exit cleanly.'}
        'exit 0'
    }
    if($Scope -eq 'feature'){Check 'shortcut-released-after-quit' {Assert-Binding 'Alt+V' $true;Assert-Binding 'Ctrl+Alt+J' $true;'test shortcuts released'}}
    $success=(@($checks|Where-Object {$_.status -eq 'FAIL'}).Count -eq 0)
}catch{Add-Check 'run' 'FAIL' $_.Exception.Message 'all assertions pass' $_.ToString();Write-Output $_.ToString()}
finally {
    foreach($name in @('release-held','stop-blocker')){[IO.File]::WriteAllText((Join-Path $EvidenceRoot $name),'stop owned fixture')}
    foreach($child in @($held,$blocker)){if($null -ne $child -and !$child.HasExited){$null=$child.WaitForExit(15000)}}
    if($browserProcess -and !$browserProcess.HasExited){try{$null=$browserProcess.CloseMainWindow();if(!$browserProcess.WaitForExit(10000)){throw 'Owned browser did not exit.'}}catch{Add-Check 'browser-cleanup' 'FAIL' $_.Exception.Message 'owned browser exits';$success=$false}}
    if($targetProcess -and !$targetProcess.HasExited){try{Target-Command $target 'shutdown'|Out-Null;if(!$targetProcess.WaitForExit(10000)){throw 'Target did not exit.'};Add-Check 'target-cleanup' 'PASS' 'owned target exited' 'graceful shutdown'}catch{Add-Check 'target-cleanup' 'FAIL' $_.Exception.Message 'graceful shutdown';$success=$false}}
    if($echoProcess -and !$echoProcess.HasExited){try{Start-ScopedProcess @('--quit');if(!$echoProcess.WaitForExit(10000)){throw 'Echo did not exit.'}}catch{Add-Check 'echo-cleanup' 'FAIL' $_.Exception.Message 'graceful shutdown';$success=$false}}
    Not-Run 'physical-ime' 'Synthetic input does not certify physical Chinese IME behavior.'
    Not-Run 'physical-mixed-dpi-matrix' 'Geometry tests cover only monitor/DPI configurations actually present during execution.'
    [ordered]@{schema='echo.focus-hotkey.acceptance.v1';status=$(if($success){'PASS'}else{'FAIL'});scope=$Scope;renderer=$Renderer;finished_utc=[DateTime]::UtcNow.ToString('o');executable_sha256=(Get-FileHash $Executable).Hash;checks=$checks.ToArray()}|ConvertTo-Json -Depth 20|Set-Content (Join-Path $EvidenceRoot 'summary.json') -Encoding utf8NoBOM
    foreach($name in $old.Keys){[Environment]::SetEnvironmentVariable($name,$old[$name],'Process')}
}
if(!$success){exit 1}
