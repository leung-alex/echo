#requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Fixture,
    [Parameter(Mandatory)][string]$OutputDirectory
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if(Test-Path -LiteralPath $OutputDirectory){throw 'Evidence directory must be new'}
if(@(Get-Process echo-desktop,Echo -ErrorAction SilentlyContinue).Count){throw 'An Echo process already exists; preserve it and stop this isolated run'}
$marker=Get-Content (Join-Path $Fixture 'synthetic-fixture.json') -Raw|ConvertFrom-Json
if(!$marker.synthetic -or $marker.capture_enabled -or $marker.history -ne 2000 -or $marker.favorites -ne 200 -or $marker.images -notin @(0,20)){throw 'Requires the approved ordinary synthetic dataset'}
New-Item -ItemType Directory -Path $OutputDirectory|Out-Null
$OutputDirectory=(Resolve-Path $OutputDirectory).Path;$Executable=(Resolve-Path $Executable).Path
Copy-Item -LiteralPath $Fixture -Destination (Join-Path $OutputDirectory 'data') -Recurse
Add-Type -Path (Join-Path $PSScriptRoot 'NativeProbe.cs')
Add-Type -Path (Join-Path $PSScriptRoot '../../tools/perf-native/ProcessSnapshot.cs')
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
$psi=[Diagnostics.ProcessStartInfo]::new($Executable);$psi.UseShellExecute=$false
$psi.RedirectStandardError=$true;$psi.RedirectStandardOutput=$true;$psi.Arguments='--history'
$psi.Environment['ECHO_DATA_DIR']=Join-Path $OutputDirectory 'data'
$psi.Environment['ECHO_WINDOWS_ACCEPTANCE']='1';$psi.Environment['ECHO_MEMORY_TRACE_DIR']=$OutputDirectory
foreach($name in @('ECHO_RENDERER','SLINT_BACKEND','SLINT_DESTROY_WINDOW_ON_HIDE','ECHO_NATIVE_TEST_ROOT','ECHO_MEMORY_NO_OFFSCREEN','ECHO_MEMORY_DESTROY_GRAPHICS','ECHO_ACCEPTANCE_FORCE_GRAPHICS_FALLBACK')){$psi.Environment.Remove($name)|Out-Null}
$rows=[Collections.Generic.List[object]]::new();$errors=[Collections.Generic.List[string]]::new();$actions=[Collections.Generic.List[object]]::new()
$result=[ordered]@{schema='echo.software-deck.memory.v1';status='RUNNING';threshold_bytes=50000000;fixture=$marker;sha256=(Get-FileHash $Executable).Hash;measurement='Private Bytes; 1 Hz sampled peaks, not allocation high-water proof';native_test=$false;gpu='No GPU renderer; compositor GPU memory is reported separately when measured';error=$null}
$app=$null
function Window {
    if($app.HasExited){throw 'Root process exited'}
    $w=@([EchoProbeNative]::Windows($app.Id)|Where-Object {$_.Visible -and $_.Title -eq 'Echo Recall'})
    if($w.Count -ne 1){throw 'Expected one owned visible window'}
    return $w[0]
}
function Tree {
    $w=Window
    return [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$w.Handle)
}
function Content {
    $elements=(Tree).FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
    foreach($e in $elements){if($e.Current.Name.Contains('echo-perf-text-1999') -and !$e.Current.IsOffscreen){return}}
    throw 'Expected complete synthetic History content is missing'
}
function Wait-Content {
    $watch=[Diagnostics.Stopwatch]::StartNew()
    while($watch.Elapsed.TotalSeconds -lt 15){try{Content;return}catch{if($app.HasExited){throw};Start-Sleep -Milliseconds 50}}
    throw 'Synthetic content did not become ready'
}
function Activate([string]$Flag){
    $start=[Diagnostics.ProcessStartInfo]::new($Executable);$start.UseShellExecute=$false;$start.Arguments=$Flag
    $start.Environment['ECHO_DATA_DIR']=$psi.Environment['ECHO_DATA_DIR']
    $child=[Diagnostics.Process]::Start($start)
    try{if(!$child.WaitForExit(10000) -or $child.ExitCode -ne 0){throw "Public activation failed: $Flag"}}finally{$child.Dispose()}
}
function Invoke-Side {
    $tree=Tree;$nodes=$tree.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
    $side=$null
    foreach($node in $nodes){if($node.Current.Name.StartsWith('Switch to ') -and !$node.Current.IsOffscreen -and $node.Current.IsEnabled){$side=$node}}
    if($null -eq $side){throw 'No enabled metadata navigation card'}
    $pattern=$null
    if(!$side.TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern,[ref]$pattern)){throw 'Side card has no native invoke action'}
    $label=$side.Current.Name;([Windows.Automation.InvokePattern]$pattern).Invoke()
    $actions.Add(@{utc=[DateTime]::UtcNow.ToString('o');action='metadata-card';target=$label})
}
function Sample([string]$Phase,[int]$Seconds){
    $watch=[Diagnostics.Stopwatch]::StartNew()
    for($i=0;$i -lt $Seconds;$i++){
        if($app.HasExited){throw 'Root exited during sampling'}
        $lag=$watch.Elapsed.TotalMilliseconds-$i*1000
        if($lag -gt 500){$errors.Add("Missing timely 1 Hz sample: $Phase/$i lag ${lag}ms")}
        $inventory=@([Echo.Performance.ProcessSnapshot]::Capture()|Where-Object {$_.Name -in @('echo-desktop.exe','Echo.exe') -or $_.ParentProcessId -eq $app.Id})
        if($inventory.Count -ne 1 -or $inventory[0].ProcessId -ne $app.Id -or $inventory[0].IdentityError -ne 0 -or $inventory[0].CreationDate.Ticks -ne $created -or $inventory[0].ExecutablePath -ne $Executable){throw 'Unknown, missing or changed product process identity'}
        $memory=[Echo.Performance.ProcessSnapshot]::ReadMemory($app.Id,$created);$app.Refresh()
        $row=[ordered]@{utc_ns=([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()*1000000).ToString();phase=$Phase;index=$i;elapsed_seconds=$watch.Elapsed.TotalSeconds;pid=$app.Id;created_ticks=$created;private_bytes=$memory.PrivateUsage.ToUInt64();working_set=$memory.WorkingSet.ToUInt64();cpu_seconds=$app.TotalProcessorTime.TotalSeconds;classification='pending';valid=($lag -le 500)}
        $rows.Add($row);$row|ConvertTo-Json -Compress|Add-Content (Join-Path $OutputDirectory 'samples.jsonl')
        # Actions follow a sample so their cost cannot masquerade as a lower reading.
        if(($Phase -eq 'visible' -and $i -in @(10,20,30,40,50,60,70,80,90,100)) -or ($Phase -eq 'restored' -and $i -in @(10,20,30,40))){Invoke-Side}
        $remaining=($i+1)*1000-$watch.Elapsed.TotalMilliseconds
        if($remaining -gt 0){Start-Sleep -Milliseconds ([int]$remaining)}
    }
}
try{
    $env:PYTHONUTF8='1'
    python (Join-Path $PSScriptRoot 'database-signature.py') (Join-Path $OutputDirectory 'data/echo.sqlite3')|Set-Content (Join-Path $OutputDirectory 'original-before.json')
    if($LASTEXITCODE -ne 0){throw 'Original representation verification failed'}
    $app=[Diagnostics.Process]::Start($psi);$stderr=$app.StandardError.ReadToEndAsync();$stdout=$app.StandardOutput.ReadToEndAsync()
    $created=$app.StartTime.ToUniversalTime().Ticks;$result.pid=$app.Id;$result.created_ticks=$created
    $watch=[Diagnostics.Stopwatch]::StartNew();$ready=$false
    while($watch.Elapsed.TotalSeconds -lt 15){try{Content;$ready=$true;break}catch{if($app.HasExited){throw};Start-Sleep -Milliseconds 100}}
    if(!$ready){throw 'Initial synthetic content did not become ready'}
    $result.window=(Window).Bounds;$result.started_utc=[DateTime]::UtcNow.ToString('o')
    Sample 'visible' 120
    $w=Window;[EchoProbeNative]::PostMessage([IntPtr]$w.Handle,0x10,[IntPtr]::Zero,[IntPtr]::Zero)|Out-Null
    Sample 'hidden' 120
    Activate '--history';Wait-Content
    Sample 'restored' 60
    $trace=@(Get-Content (Join-Path $OutputDirectory "lifecycle-$($app.Id).jsonl")|ForEach-Object {$_|ConvertFrom-Json})
    $renderer=@($trace|Where-Object state -eq 'graphics_selected')
    if($renderer.Count -ne 1 -or $renderer[0].details.renderer -ne 'software' -or $renderer[0].details.perspective){throw 'Actual software renderer was not verified'}
    $result.renderer=$renderer[0].details
    $hide=@($trace|Where-Object state -eq 'hidden_warm')[-1];$ack=@($trace|Where-Object state -eq 'hidden_reclaimed')[-1]
    $delay=([decimal]$ack.utc_ns-[decimal]$hide.utc_ns)/1000000000;$result.reclaim_seconds=$delay
    if($delay -lt 30 -or $delay -gt 35 -or $hide.details.epoch -ne $ack.details.epoch -or $hide.details.hidden_generation -ne $ack.details.hidden_generation -or $ack.details.rows -ne 0 -or $ack.details.thumbnail_bytes -ne 0 -or $ack.details.software_frame_bytes -ne 0 -or !$ack.details.worker_cache_acknowledged){throw 'Reclamation acknowledgment or 30–35 second deadline failed'}
    $relevant=@($trace|Where-Object {$_.state -in @('slide_loading','slide_started','slide_finished','view_ready','hidden_warm')})
    foreach($row in $rows){
        if($row.phase -eq 'hidden'){$row.classification=if(([decimal]$row.utc_ns-[decimal]$hide.utc_ns)/1000000000 -ge 35){'stable-hidden'}else{'warm-hidden'}}
        else{
            $events=@($relevant|Where-Object {[decimal]$_.utc_ns -le [decimal]$row.utc_ns})
            $last=if($events.Count){$events[-1].state}else{'view_ready'}
            $row.classification=if($last -in @('slide_loading','slide_started')){'transient'}else{'stable-visible'}
        }
        if($row.classification.StartsWith('stable') -and $row.private_bytes -ge 50000000){$errors.Add("Memory threshold: $($row.phase)/$($row.index) = $($row.private_bytes)")}
    }
    if($rows.Count -ne 300){$errors.Add('Expected exactly 300 valid 1 Hz samples')}
    $visible=@($rows|Where-Object classification -eq 'stable-visible');$hidden=@($rows|Where-Object classification -eq 'stable-hidden')
    if($visible.Count -lt 140 -or $hidden.Count -lt 84){$errors.Add('Too few stable samples or content never settled')}
    $result.visible=@{samples=$visible.Count;max_bytes=($visible.private_bytes|Measure-Object -Maximum).Maximum}
    $result.hidden=@{samples=$hidden.Count;max_bytes=($hidden.private_bytes|Measure-Object -Maximum).Maximum}
    $result.sampled_peak_bytes=($rows.private_bytes|Measure-Object -Maximum).Maximum
    $result.cpu_seconds=$rows[-1].cpu_seconds-$rows[0].cpu_seconds
    $intervals=[Collections.Generic.List[double]]::new();$frameCost=[Collections.Generic.List[double]]::new();$moving=$false;$lastFrame=$null
    foreach($event in $trace){
        if($event.state -eq 'slide_started'){$moving=$event.details.duration_ms -gt 0;$lastFrame=$null;if(!$event.details.prepared_frame){$errors.Add('Motion started without a prepared-content frame')}}
        if($event.state -eq 'slide_finished'){$moving=$false}
        if($event.state -eq 'frame_presented' -and $moving){
            if($null -ne $lastFrame){$intervals.Add([double](([decimal]$event.utc_ns-$lastFrame)/1000000))}
            $frameCost.Add($event.details.render_present_us/1000);$lastFrame=[decimal]$event.utc_ns
        }
        if($event.state -eq 'display_data' -and ($event.details.page_bytes+$event.details.row_model_bytes+$event.details.side_bytes+$event.details.outgoing_bytes+$event.details.queued_bytes) -gt 4*1024*1024){$errors.Add('Observed model and in-flight display data exceeded 4 MiB')}
        if($event.state -eq 'oversized_display_result'){$errors.Add('Oversized display result invalidates the ordinary-cache budget gate')}
    }
    $sorted=@($intervals|Sort-Object)
    if($sorted.Count -lt 30){$errors.Add('Too few presented animation frames')}
    else{
        $result.animation=@{presented_intervals=$sorted.Count;p95_ms=$sorted[[Math]::Ceiling($sorted.Count*.95)-1];max_ms=$sorted[-1];render_present_max_ms=($frameCost|Measure-Object -Maximum).Maximum}
        if($result.animation.p95_ms -gt 20){$errors.Add("Animation P95 exceeded 20ms: $($result.animation.p95_ms)")}
    }
    $result.status=if($errors.Count){'FAIL'}else{'PASS'}
}catch{$result.status='FAIL';$result.error=$_.ToString()}
finally{
    if($null -ne $app -and !$app.HasExited){Activate '--quit';if(!$app.WaitForExit(10000)){$result.status='FAIL';$errors.Add('Root did not exit gracefully')}}
    if($null -ne $app -and $app.HasExited){$result.exit_code=$app.ExitCode;if($app.ExitCode -ne 0){$result.status='FAIL'};$stderr.Result|Set-Content (Join-Path $OutputDirectory 'stderr.log');$stdout.Result|Set-Content (Join-Path $OutputDirectory 'stdout.log')}
    python (Join-Path $PSScriptRoot 'database-signature.py') (Join-Path $OutputDirectory 'data/echo.sqlite3')|Set-Content (Join-Path $OutputDirectory 'original-after.json')
    if($LASTEXITCODE -ne 0 -or (Get-Content (Join-Path $OutputDirectory 'original-before.json') -Raw) -ne (Get-Content (Join-Path $OutputDirectory 'original-after.json') -Raw)){$result.status='FAIL';$errors.Add('Original representation signature changed')}
    $rows|ConvertTo-Json -Depth 8|Set-Content (Join-Path $OutputDirectory 'classified-samples.json')
    $actions|ConvertTo-Json -Depth 8|Set-Content (Join-Path $OutputDirectory 'actions.json')
    $result.errors=@($errors);$result.finished_utc=[DateTime]::UtcNow.ToString('o')
    $result|ConvertTo-Json -Depth 20|Set-Content (Join-Path $OutputDirectory 'result.json');$result|ConvertTo-Json -Depth 8
}
if($result.status -ne 'PASS'){exit 1}
