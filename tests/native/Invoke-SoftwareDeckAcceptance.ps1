[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Template,
    [Parameter(Mandatory)][string]$EvidenceRoot,
    [ValidateSet('software')][string]$Renderer='software',
    [switch]$CoreOnly
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$OutputEncoding=[Console]::OutputEncoding
if ($env:ECHO_WINDOWS_ACCEPTANCE -ne '1') {throw 'Explicit native acceptance authorization is required.'}
$Root=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Root)
$Executable=[IO.Path]::GetFullPath($Executable,$Root)
$EvidenceRoot=[IO.Path]::GetFullPath($EvidenceRoot,$Root)
$Template=[IO.Path]::GetFullPath($Template,$Root)
$marker=Get-Content -LiteralPath (Join-Path $Template 'synthetic-fixture.json') -Raw | ConvertFrom-Json
if (!$marker.synthetic -or $marker.capture_enabled) {throw 'Only a capture-disabled synthetic fixture is allowed.'}
if (Test-Path -LiteralPath $EvidenceRoot) {throw 'Evidence directory must be new.'}
New-Item -ItemType Directory $EvidenceRoot | Out-Null
Copy-Item -LiteralPath $Template -Destination (Join-Path $EvidenceRoot 'data') -Recurse
$driver=Join-Path $EvidenceRoot 'EchoDriver.exe'
$framework=Join-Path $env:WINDIR 'Microsoft.NET/Framework64/v4.0.30319'
& (Join-Path $framework 'csc.exe') /nologo /target:exe "/out:$driver" "/reference:$framework/WPF/UIAutomationClient.dll" "/reference:$framework/WPF/UIAutomationTypes.dll" "/reference:$framework/WPF/WindowsBase.dll" /reference:System.Drawing.dll /reference:System.Windows.Forms.dll /reference:System.Web.Extensions.dll (Join-Path $Root 'tests/native/EchoUi.cs') (Join-Path $Root 'tests/native/EchoDriver.cs') (Join-Path $Root 'tests/native/EchoBenchmarks.cs') (Join-Path $Root 'tests/native/EchoComposition.cs')
if ($LASTEXITCODE -ne 0) {throw 'Native driver compilation failed.'}
$checks=[Collections.Generic.List[object]]::new()
$calls=[Collections.Generic.List[object]]::new()
$owned=$null
$old=@{}
foreach($key in @('ECHO_DATA_DIR','ECHO_RENDERER','ECHO_ACCEPTANCE_RUN_ROOT','ECHO_NATIVE_TEST_ROOT','RUST_BACKTRACE','ECHO_MEMORY_TRACE_DIR','ECHO_SOFTWARE_ALPHA_PRESENTATION')) {$old[$key]=[Environment]::GetEnvironmentVariable($key,'Process')}
function Ui([string]$Operation,[string[]]$Arguments=@()) {
    if($null -eq $owned -or $owned.HasExited){throw 'Owned Echo process exited before a native operation.'}
    if($owned.StartTime.ToUniversalTime().Ticks -ne $ownedCreationTicks -or
        ![StringComparer]::OrdinalIgnoreCase.Equals($owned.MainModule.FileName,$Executable)) {
        throw 'Owned Echo process identity changed before a native operation.'
    }
    $watch=[Diagnostics.Stopwatch]::StartNew()
    $start=[Diagnostics.ProcessStartInfo]::new($driver)
    $start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
    $start.StandardOutputEncoding=[Text.UTF8Encoding]::new($false);$start.StandardErrorEncoding=[Text.UTF8Encoding]::new($false)
    foreach($argument in @($Operation,[string]$owned.Id,'Echo Recall') + $Arguments){$start.ArgumentList.Add($argument)}
    $process=[Diagnostics.Process]::new();$process.StartInfo=$start
    try {
        $null=$process.Start();$stdout=$process.StandardOutput.ReadToEndAsync();$stderr=$process.StandardError.ReadToEndAsync()
        if(!$process.WaitForExit(20000)){$process.Kill($true);throw "Owned test driver timed out: $Operation"}
        $code=$process.ExitCode;$text=($stdout.Result+$stderr.Result).Trim().Trim([char]0xfeff)
    }finally{$process.Dispose()}
    $calls.Add([ordered]@{operation=$Operation;arguments=$Arguments;exit=$code;elapsed_ms=$watch.Elapsed.TotalMilliseconds;response=$text})
    if($code -ne 0){throw "Native operation failed: $Operation $text"}
    $value=$text | ConvertFrom-Json
    return $value.value
}
function Ensure-Control([string]$Label) {
    for($attempt=0;$attempt -lt 12;$attempt++) {
        $dump=[string](Ui 'dump')
        if($dump.Contains("| $Label |")){return}
        Ui 'scroll' @('260') | Out-Null
        Start-Sleep -Milliseconds 35
    }
    throw "The control could not be reached by scrolling: $Label"
}
function Invoke-Ui([string]$Label) {Ui 'invoke' @($Label) | Out-Null}
function Set-Ui([string]$Label,[string]$Value) {Ui 'value' @($Label,$Value) | Out-Null}
function Wait-Text([string]$Text) {
    $watch=[Diagnostics.Stopwatch]::StartNew()
    do {
        $dump=[string](Ui 'dump')
        if($dump.Contains($Text)){return $dump}
        Start-Sleep -Milliseconds 40
    }while($watch.ElapsedMilliseconds -lt 10000)
    throw "Expected native content did not appear: $Text"
}
function Check([string]$Name,[scriptblock]$Test) {
    $watch=[Diagnostics.Stopwatch]::StartNew()
    try {& $Test; $checks.Add([ordered]@{name=$Name;status='PASS';elapsed_ms=$watch.Elapsed.TotalMilliseconds});Write-Output "PASS $Name"}
    catch {$checks.Add([ordered]@{name=$Name;status='FAIL';error=$_.ToString()});throw}
}
function Not-Run([string]$Name,[string]$Reason) {
    $checks.Add([ordered]@{name=$Name;status='NOT_RUN';reason=$Reason})
    Write-Output "NOT_RUN $Name"
}
function Focus-Surface {
    # F6 is the public manager focus-mode toggle. The suite starts in content
    # focus and every call restores that state after a bounded two-toggle cycle.
    Ui 'key' @('117') | Out-Null
    Start-Sleep -Milliseconds 25
    Ui 'key' @('117') | Out-Null
}
function Wait-SpaceReady {
    $watch=[Diagnostics.Stopwatch]::StartNew()
    do {
        $state=Ui 'metrics'
        if($state.phase -eq 'Idle' -and $state.ready -and !$state.loading -and
            [string]$state.requested -eq [string]$state.space -and
            [string]$state.presented -eq [string]$state.space -and
            [string]$state.interaction -eq [string]$state.space){return}
        Start-Sleep -Milliseconds 25
    }while($watch.ElapsedMilliseconds -lt 10000)
    throw 'The requested space did not become interactive.'
}
function Wait-SpaceTarget([int]$Id,[string]$Title) {
    $watch=[Diagnostics.Stopwatch]::StartNew()
    do {
        $state=Ui 'metrics'
        $space=@($state.spaces | Where-Object { [int]$_.id -eq $Id -and [string]$_.title -eq $Title })
        if([int]$state.space -eq $Id -and [string]$state.route -eq 'history' -and
            [string]$state.phase -eq 'Idle' -and $state.ready -and !$state.loading -and $space.Count -eq 1 -and
            [string]$state.requested -eq [string]$Id -and [string]$state.presented -eq [string]$Id -and
            [string]$state.interaction -eq [string]$Id){return}
        Start-Sleep -Milliseconds 25
    }while($watch.ElapsedMilliseconds -lt 10000)
    throw "The requested space did not become interactive: $Title ($Id)"
}
function Shot([string]$Name) {
    $watch=[Diagnostics.Stopwatch]::StartNew()
    while((Ui 'metrics').phase -eq 'Animating'){
        if($watch.ElapsedMilliseconds -gt 3000){throw 'Motion did not settle before screenshot.'}
        Start-Sleep -Milliseconds 25
    }
    Ui 'capture' @((Join-Path $EvidenceRoot "$Name.png")) | Out-Null
}
function Bridge([string]$Verb,[int]$Key=0,[bool]$Shift=$false,[bool]$Paused=$false,[string]$File='') {
    if($owned.HasExited -or $owned.StartTime.ToUniversalTime().Ticks -ne $ownedCreationTicks){throw 'Owned process identity unavailable'}
    $id=[Guid]::NewGuid().ToString('N');$control=Join-Path $EvidenceRoot 'native-control'
    [IO.File]::WriteAllText((Join-Path $control 'request.pending'),(@{id=$id;pid=$owned.Id;verb=$Verb;key=$Key;shift=$Shift;ctrl=$false;paused=$Paused;file=$File}|ConvertTo-Json -Compress))
    Move-Item -LiteralPath (Join-Path $control 'request.pending') -Destination (Join-Path $control 'request.json') -Force
    $watch=[Diagnostics.Stopwatch]::StartNew()
    while($watch.Elapsed.TotalSeconds -lt 12){
        if($owned.HasExited){throw 'Application exited awaiting test response'}
        $response=Join-Path $control 'response.json';$r=$null
        if(Test-Path $response){try{$r=Get-Content $response -Raw | ConvertFrom-Json}catch{}}
        if($null -ne $r -and $r.id -eq $id){if($r.status -ne 'PASS'){throw $r.error};return $r.value}
        Start-Sleep -Milliseconds 5
    }
    throw "Bridge timeout: $Verb"
}
function Ready {
    $watch=[Diagnostics.Stopwatch]::StartNew()
    do{
        $m=Bridge 'metrics'
        $sidesReady=@($m.side_previews|Where-Object {$_.visible -and (!$_.ready -or $_.loading -or $_.query -ne $m.query)}).Count -eq 0
        if($m.phase -eq 'Idle' -and $m.ready -and !$m.loading -and $m.requested -eq $m.presented -and $m.interaction -eq $m.requested -and !$m.software_slide.outgoing -and !$m.flow_timer -and $sidesReady){
            if($m.display_bytes.total -gt 4*1024*1024 -or $m.thumbnails_bytes -gt 4*1024*1024){throw 'Display cache budget exceeded'}
            return $m
        }
        Start-Sleep -Milliseconds 20
    }while($watch.Elapsed.TotalSeconds -lt 12)
    $m|ConvertTo-Json -Depth 20|Set-Content (Join-Path $EvidenceRoot 'not-ready.json')
    throw 'Requested content did not become ready with the animation timer stopped'
}
function Open-History {
    & $Executable --history | Out-Null
    if($LASTEXITCODE -ne 0){throw 'Public activation failed'}
    return Ready
}
$success=$false
try{
    $env:PYTHONUTF8='1'
    python (Join-Path $Root 'tests/performance/database-signature.py') (Join-Path $EvidenceRoot 'data/echo.sqlite3')|Set-Content (Join-Path $EvidenceRoot 'original-before.json')
    if($LASTEXITCODE -ne 0){throw 'Original representation verification failed'}
    $env:ECHO_DATA_DIR=Join-Path $EvidenceRoot 'data';$env:ECHO_RENDERER='software'
    $env:ECHO_ACCEPTANCE_RUN_ROOT=$EvidenceRoot;$env:ECHO_NATIVE_TEST_ROOT=$EvidenceRoot
    $env:ECHO_MEMORY_TRACE_DIR=$EvidenceRoot;$env:RUST_BACKTRACE='1'
    $env:ECHO_SOFTWARE_ALPHA_PRESENTATION='1'
    $owned=Start-Process -FilePath $Executable -ArgumentList '--history' -PassThru -WindowStyle Hidden -RedirectStandardError (Join-Path $EvidenceRoot 'application-stderr.log') -RedirectStandardOutput (Join-Path $EvidenceRoot 'application-stdout.log')
    $ownedCreationTicks=$owned.StartTime.ToUniversalTime().Ticks
    $owned.Id|Set-Content (Join-Path $EvidenceRoot 'owned-pid.txt')
    Start-Sleep -Milliseconds 1500
    Check 'software-single-window-complete-content' {
        $m=Ready
        if($m.renderer -ne 'software' -or $null -ne $m.graphics -or $m.snapshot_model_count -lt 1){throw 'Software content is not ready'}
        if((Ui 'window-count') -ne 1){throw 'Expected one window'}
        if([Math]::Abs($m.software_slide.side_width-$m.panel[2]*0.7) -gt 0.5){throw 'Side width differs from the approved old-version proportion'}
        $m|ConvertTo-Json -Depth 20|Set-Content (Join-Path $EvidenceRoot 'initial-metrics.json')
        Ui 'dump'|Set-Content (Join-Path $EvidenceRoot 'initial-uia.txt')
        Shot '01-history'
    }
    if(!$CoreOnly){
        Check 'enter-blocked-during-load-and-motion' {
            if($env:ECHO_CLIPBOARD_BACKUP_READY -ne '1'){throw 'Input-safety regression requires the clipboard preservation wrapper'}
            Bridge 'search_fault' 400|Out-Null;Bridge 'step'|Out-Null
            Bridge 'key' 13|Out-Null;$m=Bridge 'metrics'
            if($m.phase -eq 'Idle' -or !$m.software_slide.loading){throw 'Enter bypassed the loading barrier'}
            Ready|Out-Null;Bridge 'step'|Out-Null
            $watch=[Diagnostics.Stopwatch]::StartNew()
            do{$m=Bridge 'metrics'}while(!$m.software_slide.moving -and $watch.ElapsedMilliseconds -lt 1500)
            if(!$m.software_slide.moving){throw 'Did not observe active motion'}
            Bridge 'key' 13|Out-Null;$m=Bridge 'metrics'
            if(!$m.visible -or $m.phase -ne 'Animating' -or !$m.software_slide.outgoing){throw 'Enter finished or executed during active motion'}
            $m|ConvertTo-Json -Depth 20|Set-Content (Join-Path $EvidenceRoot 'carousel-motion.json')
            # Shot deliberately waits for idle. Capture this active frame through
            # the existing in-process bridge instead of mislabelling an idle PNG.
            Bridge 'capture' -File '02-carousel-motion.png'|Out-Null
            Ready|Out-Null
        }
        Check 'thirty-space-switches-release-outgoing-and-stop-timer' {
            for($i=0;$i -lt 30;$i++){
                $before=Ready;Bridge 'step'|Out-Null;$after=Ready
                if($before.space -eq $after.space){throw "Space switch $i did not change space"}
                $after|ConvertTo-Json -Depth 20 -Compress|Add-Content (Join-Path $EvidenceRoot 'switches.jsonl')
            }
        }
        Check 'rapid-switches-keep-only-final-target' {
            $before=Ready;$spaces=@($before.spaces);$index=0
            for($i=0;$i -lt $spaces.Count;$i++){if($spaces[$i].id -eq $before.space){$index=$i}}
            for($i=0;$i -lt 7;$i++){Bridge 'step'|Out-Null}
            $after=Ready;$target=$spaces[($index+7)%$spaces.Count].id
            if($after.space -ne $target){throw "Rapid navigation ended at $($after.space), expected $target"}
        }
        Check 'hide-during-switch-and-restore' {
            Bridge 'step'|Out-Null;Ui 'close'|Out-Null;Start-Sleep -Milliseconds 100
            $m=Bridge 'metrics'
            if($m.visible -or $m.software_slide.outgoing -or $m.flow_timer){throw 'Hidden motion retained an outgoing model or timer'}
            Open-History|Out-Null
        }
        Check 'public-activation-cancels-old-carousel' {
            Bridge 'search_fault' 400|Out-Null;Bridge 'step'|Out-Null
            $m=Open-History
            if($m.space -ne 1 -or $m.software_slide.loading -or $m.software_slide.outgoing){throw 'Activation retained the previous loading carousel'}
            Bridge 'step'|Out-Null
            $watch=[Diagnostics.Stopwatch]::StartNew()
            do{$m=Bridge 'metrics'}while(!$m.software_slide.moving -and $watch.ElapsedMilliseconds -lt 1500)
            if(!$m.software_slide.moving){throw 'Did not observe active carousel for activation regression'}
            $m=Open-History
            if($m.space -ne 1 -or $m.software_slide.moving -or $m.software_slide.outgoing){throw 'Activation retained the previous moving carousel'}
        }
        Check 'metadata-side-card-click' {
            $m=Open-History;$before=$m.space
            if(!$m.software_slide.left_visible -and !$m.software_slide.right_visible){throw 'No metadata side card at normal size'}
            $dump=[string](Ui 'dump');$label=([regex]::Match($dump,'Switch to [^|\r\n]+')).Value.Trim()
            if(!$label){throw 'Side card is not accessible'}
            Invoke-Ui $label;$after=Ready
            if($after.space -eq $before){throw 'Side card click did not navigate'}
            Shot '02-side-navigation'
        }
        Check 'favorites-complete-list-and-both-neighbor-filters' {
            Open-History|Out-Null;Bridge 'step'|Out-Null;$m=Ready
            if($m.space -ne 2 -or $m.snapshot_model_count -lt 1 -or @($m.spaces|Where-Object {$_.id -eq 2})[0].count -ne 200){throw 'Favorites does not show its retained entries'}
            Shot '03-favorites-entries'
            foreach($query in @('echo-perf-text-0013','Echo favorite 199','zz-no-match-neighbor')){
                Bridge 'query' -File $query|Out-Null;$m=Ready
                $m|ConvertTo-Json -Depth 20 -Compress|Add-Content (Join-Path $EvidenceRoot 'neighbor-filters.jsonl')
                $sides=@($m.side_previews|Where-Object visible)
                if($sides.Count -ne 2){throw 'Both neighbor cards must be visible at the normal test size'}
                foreach($side in $sides){if(!$side.ready -or $side.loading -or $side.query -ne $query -or $side.rows -gt 4){throw 'Neighbor query generation or preview row bound failed'}}
                if($query -eq 'echo-perf-text-0013' -and (@($sides|Where-Object space -eq '1')[0].rows -ne 1 -or @($sides|Where-Object space -eq '3')[0].rows -ne 0)){throw 'History and Work filtered results are incorrect'}
                # Fuzzy tokens scan name + body: 199 also matches the subsequence
                # in "Meeting notes 198 ... Echo favorite 198". All ten Work rows
                # match; the exact 199 result ranks first and only four are shown.
                if($query -eq 'Echo favorite 199' -and (@($sides|Where-Object space -eq '1')[0].rows -ne 0 -or @($sides|Where-Object space -eq '3')[0].rows -ne 4 -or @($sides|Where-Object space -eq '3')[0].titles[0] -ne 'Quick reply 199')){throw 'Saved-space filter or ranking is incorrect'}
                if($query -eq 'zz-no-match-neighbor' -and ($m.snapshot_model_count -ne 0 -or @($sides|Where-Object {$_.rows -ne 0}).Count)){throw 'A stale neighbor result survived the no-match query'}
            }
            Bridge 'query' -File ''|Out-Null;Ready|Out-Null
        }
        Check 'narrow-window-hides-side-cards' {
            Ui 'resize' @('590','660')|Out-Null;Start-Sleep -Milliseconds 200;$m=Ready
            if($m.software_slide.left_visible -or $m.software_slide.right_visible){throw 'Narrow window still shows side cards'}
            Shot '03-narrow';Ui 'resize' @('1000','760')|Out-Null;Ready|Out-Null
        }
        Check 'mixed-row-scroll' {
            Open-History|Out-Null;Ui 'dump'|Set-Content (Join-Path $EvidenceRoot 'search-controls.txt')
            # The manager has no search edit; the input-query route is exercised
            # separately by the isolated inline acceptance suite.
            Bridge 'scroll' 340|Out-Null;Start-Sleep -Milliseconds 100;Shot '04-scrolled'
        }
        Check 'bounded-pages-retain-forward-and-backward-history' {
            $first=Open-History
            Bridge 'key' 34|Out-Null;$second=Ready
            Bridge 'key' 34|Out-Null;$third=Ready
            if($second.snapshot_model_count -ne 40 -or $third.snapshot_model_count -ne 20 -or $third.selection -eq $first.selection){throw 'Bounded page forward cursor failed'}
            Bridge 'key' 33|Out-Null;$back=Ready
            if($back.selection -ne $first.selection){throw 'Previous page did not restore the original cursor'}
        }
        Check 'saved-content-create-edit-icon-delete' {
            Open-History|Out-Null;Bridge 'step'|Out-Null;Ready|Out-Null
            Invoke-Ui 'New content';Wait-Text 'Create favorite'|Out-Null
            Set-Ui 'Favorite name (required)' 'Echo retirement synthetic favorite'
            Set-Ui 'Favorite content' 'echo-retirement-synthetic-content'
            Invoke-Ui 'Choose icon'
            Wait-Text 'Icon Mail'|Out-Null;Invoke-Ui 'Icon Mail'
            if((Ui 'read' @('Favorite icon')) -ne 'Mail'){throw 'Icon picker did not retain the Mail key'}
            Invoke-Ui 'Save favorite';Ready|Out-Null;Wait-Text 'Echo retirement synthetic favorite'|Out-Null
            Ui 'select' @('Echo retirement synthetic favorite: echo-retirement-synthetic-content')|Out-Null
            Invoke-Ui 'Edit saved content';Wait-Text 'Edit favorite'|Out-Null
            if((Ui 'read' @('Favorite icon')) -ne 'Mail' -or (Ui 'read' @('Favorite content')) -ne 'echo-retirement-synthetic-content'){throw 'Saved content/icon did not persist'}
            Set-Ui 'Favorite name (required)' 'Echo retirement edited favorite'
            Set-Ui 'Favorite content' 'echo-retirement-synthetic-updated'
            Invoke-Ui 'Save favorite';Ready|Out-Null;Wait-Text 'Echo retirement edited favorite'|Out-Null
            Ui 'select' @('Echo retirement edited favorite: echo-retirement-synthetic-updated')|Out-Null
            Invoke-Ui 'Edit saved content';Wait-Text 'Edit favorite'|Out-Null
            if((Ui 'read' @('Favorite content')) -ne 'echo-retirement-synthetic-updated'){throw 'Edited content did not persist'}
            Invoke-Ui 'Cancel';Ready|Out-Null
            Ui 'select' @('Echo retirement edited favorite: echo-retirement-synthetic-updated')|Out-Null
            Invoke-Ui 'Delete everywhere…';Wait-Text 'Delete saved content everywhere?'|Out-Null
            Invoke-Ui 'Delete everywhere';Ready|Out-Null
            if(([string](Ui 'dump')).Contains('Echo retirement edited favorite')){throw 'Deleted favorite is still visible'}
            Open-History|Out-Null
        }
        Check 'history-clear-cancel-retains-content' {
            Open-History|Out-Null
            Invoke-Ui 'Clear unpinned history';Wait-Text 'Clear unpinned history?'|Out-Null
            Invoke-Ui 'Cancel';Ready|Out-Null
            if(!([string](Ui 'dump')).Contains('echo-perf-text-1999')){throw 'Cancel lost the synthetic History page'}
        }
        Check 'settings-light-dark-and-software-preview' {
            Invoke-Ui 'Settings';Wait-Text 'Appearance & motion'|Out-Null
            Ui 'theme' @('light')|Out-Null;Shot '05-settings-light'
            $dump=[string](Ui 'dump');$dump|Set-Content (Join-Path $EvidenceRoot 'settings-uia.txt')
            if($dump -match 'WGPU|Skia|Reflection quality|Graphics backend'){throw 'Retired graphics controls remain'}
            Invoke-Ui 'Play once';Start-Sleep -Milliseconds 250
            Ui 'theme' @('dark')|Out-Null;Shot '06-settings-dark'
            Invoke-Ui 'Save'
            $saved=[Diagnostics.Stopwatch]::StartNew()
            do { $state=Bridge 'metrics'; if(!$state.settings.dirty){break}; Start-Sleep -Milliseconds 25 } while($saved.ElapsedMilliseconds -lt 10000)
            if($state.settings.dirty){throw 'Settings save did not complete'}
            Invoke-Ui 'Cancel';Ready|Out-Null;Shot '07-dark-card'
        }
        Check 'settings-invalid-valid-and-about' {
            Invoke-Ui 'Settings';Wait-Text 'Appearance & motion'|Out-Null
            Invoke-Ui 'Storage & diagnostics';Wait-Text 'Maximum history entries'|Out-Null
            Set-Ui 'Maximum history entries' '0'
            $invalid=Bridge 'metrics'
            if($invalid.settings.valid -or !$invalid.settings.error.Contains('between 1 and 2000')){throw 'Invalid History limit was not rejected'}
            if(!([string](Ui 'dump')).Contains('Save | enabled=False')){throw 'Invalid settings can be saved'}
            Set-Ui 'Maximum history entries' '2000';Set-Ui 'Total storage MiB' '513';Set-Ui 'Maximum item MiB' '32'
            if(!(Bridge 'metrics').settings.valid){throw 'Valid storage settings were rejected'}
            Invoke-Ui 'Save'
            $saved=[Diagnostics.Stopwatch]::StartNew()
            do {$state=Bridge 'metrics';if(!$state.settings.dirty){break};Start-Sleep -Milliseconds 25}while($saved.ElapsedMilliseconds -lt 10000)
            if($state.settings.dirty -or !$state.settings.valid){throw 'Valid settings did not save'}
            Invoke-Ui 'About Echo';Wait-Text 'Rust + Slint'|Out-Null;Shot 'about-current'
            Invoke-Ui 'Back to settings';Invoke-Ui 'Cancel';Ready|Out-Null
        }
        Check 'activation-replay-and-malformed-envelope' {
            $envelope=@{version=1;request_id=[guid]::NewGuid().ToString();action='echo.open';origin=$null;payload=@{}}
            $token=[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes(($envelope|ConvertTo-Json -Compress))).TrimEnd('=').Replace('+','-').Replace('/','_')
            foreach($attempt in 1..2){& $Executable --echo-activate $token|Out-Null;if($LASTEXITCODE){throw 'Valid activation failed'}}
            Ready|Out-Null
            & $Executable --echo-activate 'not-base64url' 2> (Join-Path $EvidenceRoot 'malformed-activation.log')|Out-Null
            if($LASTEXITCODE -eq 0){throw 'Malformed activation accepted'}
            if($owned.HasExited -or (Ui 'window-count') -ne 1){throw 'Activation replaced or duplicated the resident'}
            Ready|Out-Null
        }
        Check 'deep-hide-reclaims-models-images-and-frame' {
            Ui 'close'|Out-Null
            for($i=0;$i -lt 35;$i++){Start-Sleep -Seconds 1;if($owned.HasExited){throw 'Root exited during hidden period'}}
            $m=Bridge 'metrics';$m|ConvertTo-Json -Depth 20|Set-Content (Join-Path $EvidenceRoot 'reclaimed-metrics.json')
            if($m.visible -or $m.snapshot_model_count -ne 0 -or $m.thumbnails_bytes -ne 0 -or $m.software_slide.frame_bytes -ne 0 -or $m.flow_timer){throw 'Hidden display resources remain'}
            Open-History|Out-Null;Shot '08-restored'
        }
        Check 'native-alpha-over-owned-underlay' {
            Open-History|Out-Null;Ui 'resize' @('1600','800')|Out-Null;Ready|Out-Null
            Shot '09-composition-history'
            Ui 'composition' @((Join-Path $EvidenceRoot '09-composition-history.png'))|ConvertTo-Json -Depth 12|Set-Content (Join-Path $EvidenceRoot 'composition.json')
        }
    }
    if(!$CoreOnly){
        Check 'settings-and-deletion-persist-after-restart' {
            & $Executable --quit|Out-Null
            if(!$owned.WaitForExit(10000) -or $owned.ExitCode -ne 0){throw 'First resident failed to exit'}
            $owned.Dispose()
            $script:owned=Start-Process -FilePath $Executable -ArgumentList '--history' -PassThru -WindowStyle Hidden -RedirectStandardError (Join-Path $EvidenceRoot 'restart-stderr.log') -RedirectStandardOutput (Join-Path $EvidenceRoot 'restart-stdout.log')
            $script:ownedCreationTicks=$owned.StartTime.ToUniversalTime().Ticks
            $owned.Id|Set-Content (Join-Path $EvidenceRoot 'restart-pid.txt')
            Start-Sleep -Milliseconds 1500;Ready|Out-Null
            Invoke-Ui 'Settings';Invoke-Ui 'Storage & diagnostics';Wait-Text 'Maximum history entries'|Out-Null
            if((Ui 'read' @('Maximum history entries')) -ne '2000' -or (Ui 'read' @('Total storage MiB')) -ne '513' -or (Ui 'read' @('Maximum item MiB')) -ne '32'){throw 'Saved storage settings did not survive restart'}
            Invoke-Ui 'Cancel';Open-History|Out-Null;Bridge 'step'|Out-Null;Ready|Out-Null
            if(([string](Ui 'dump')).Contains('Echo retirement edited favorite')){throw 'Deleted saved item returned after restart'}
            Open-History|Out-Null
        }
    }
    $success=$true
}catch{
    $checks.Add(@{name='run';status='FAIL';error=$_.ToString()});Write-Output $_.ToString()
}finally{
    if($null -ne $owned -and !$owned.HasExited){
        & $Executable --quit
        if(!$owned.WaitForExit(10000) -or $owned.ExitCode -ne 0){$success=$false;$checks.Add(@{name='graceful-exit';status='FAIL'})}
    }
    python (Join-Path $Root 'tests/performance/database-signature.py') (Join-Path $EvidenceRoot 'data/echo.sqlite3')|Set-Content (Join-Path $EvidenceRoot 'original-after.json')
    $originalsMatch=$LASTEXITCODE -eq 0 -and (Get-Content (Join-Path $EvidenceRoot 'original-before.json') -Raw) -eq (Get-Content (Join-Path $EvidenceRoot 'original-after.json') -Raw)
    if(!$CoreOnly -and @($checks|Where-Object {$_.name -eq 'saved-content-create-edit-icon-delete' -and $_.status -eq 'PASS'}).Count -eq 1){
        python (Join-Path $Root 'tests/native/verify-software-originals.py') $Template $EvidenceRoot|Set-Content (Join-Path $EvidenceRoot 'mutation-originals-verification.json')
        $originalsMatch=$LASTEXITCODE -eq 0
    }
    if(!$originalsMatch){$success=$false;$checks.Add(@{name='retained-original-representations';status='FAIL'})}
    else{$checks.Add(@{name='retained-original-representations';status='PASS'})}
    $stderr=(@(Get-ChildItem -LiteralPath $EvidenceRoot -Filter '*stderr.log' -File)|ForEach-Object {Get-Content -LiteralPath $_.FullName -Raw}) -join "`n"
    if($stderr -match 'panicked|Present software window:|Create software presentation'){$success=$false;$checks.Add(@{name='software-presentation-errors';status='FAIL'})}
    foreach($name in @('physical-ime','multiple-monitors')){Not-Run $name 'Requires a separate physical environment'}
    $checks|ConvertTo-Json -Depth 20|Set-Content (Join-Path $EvidenceRoot 'checks.json') -Encoding utf8NoBOM
    $calls|ConvertTo-Json -Depth 20|Set-Content (Join-Path $EvidenceRoot 'calls.json') -Encoding utf8NoBOM
    @{schema='echo.software-deck.acceptance.v1';status=$(if($success){'PASS'}else{'FAIL'});executable=$Executable;sha256=(Get-FileHash $Executable).Hash;pid=$owned.Id;created_ticks=$ownedCreationTicks}|ConvertTo-Json|Set-Content (Join-Path $EvidenceRoot 'summary.json') -Encoding utf8NoBOM
    foreach($key in $old.Keys){[Environment]::SetEnvironmentVariable($key,$old[$key],'Process')}
}
if(!$success){exit 1}
