[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Template,
    [Parameter(Mandatory)][string]$EvidenceRoot,
    [ValidateSet('software','femtovg-wgpu')][string]$Renderer='femtovg-wgpu',
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
foreach($key in @('ECHO_DATA_DIR','ECHO_RENDERER','ECHO_ACCEPTANCE_RUN_ROOT','ECHO_NATIVE_TEST_ROOT','RUST_BACKTRACE')) {$old[$key]=[Environment]::GetEnvironmentVariable($key,'Process')}
function Ui([string]$Operation,[string[]]$Arguments=@()) {
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
function Wait-SpaceReady {
    $watch=[Diagnostics.Stopwatch]::StartNew()
    do {
        $state=Ui 'metrics'
        if($state.phase -eq 'Idle' -and $state.ready -and !$state.loading){return}
        Start-Sleep -Milliseconds 25
    }while($watch.ElapsedMilliseconds -lt 10000)
    throw 'The requested space did not become interactive.'
}
function Shot([string]$Name) {
    $watch=[Diagnostics.Stopwatch]::StartNew()
    while((Ui 'metrics').phase -eq 'Animating'){
        if($watch.ElapsedMilliseconds -gt 3000){throw 'Motion did not settle before screenshot.'}
        Start-Sleep -Milliseconds 25
    }
    Ui 'capture' @((Join-Path $EvidenceRoot "$Name.png")) | Out-Null
}
function Pick-Prefix([string]$Prefix) {
    $dump=[string](Ui 'dump')
    $line=@($dump -split "`n" | Where-Object {$_ -like "ControlType.Button | $Prefix*"})[0]
    if(!$line){throw "Picker option not found: $Prefix"}
    $name=($line -split ' \| ')[1]
    Invoke-Ui $name
}
function Open-Space([string]$Title) {
    Invoke-Ui 'Choose space'
    Wait-Text 'Choose space' | Out-Null
    Pick-Prefix $Title
    Wait-Text "ControlType.Group | $Title space" | Out-Null
    Wait-SpaceReady
}
$success=$false
try {
    $env:ECHO_DATA_DIR=Join-Path $EvidenceRoot 'data'
    $env:ECHO_RENDERER=$Renderer
    $env:ECHO_ACCEPTANCE_RUN_ROOT=$EvidenceRoot
    $env:ECHO_NATIVE_TEST_ROOT=$EvidenceRoot
    $env:RUST_BACKTRACE='1'
    $owned=Start-Process -FilePath $Executable -ArgumentList '--history' -PassThru -RedirectStandardError (Join-Path $EvidenceRoot 'application-stderr.log') -RedirectStandardOutput (Join-Path $EvidenceRoot 'application-stdout.log')
    $owned.Id | Set-Content (Join-Path $EvidenceRoot 'owned-pid.txt')
    Start-Sleep -Milliseconds 1500
    Check 'one-window-history-ready' {
        Wait-Text 'ControlType.Group | History space' | Out-Null
        if((Ui 'window-count') -ne 1){throw 'More than one Echo top-level window was shown.'}
        Shot '01-history'
    }
    Check 'native-card-transparency-and-composition' {
        Wait-SpaceReady
        $attempts=[Collections.Generic.List[object]]::new()
        for($attempt=1;$attempt -le 3;$attempt++) {
            try {
                $proof=Ui 'composition' @((Join-Path $EvidenceRoot '01-history.png'))
                $attempts.Add([ordered]@{attempt=$attempt;status='PASS'})
                break
            } catch {
                $attempts.Add([ordered]@{attempt=$attempt;status='INTERRUPTED';reason=$_.ToString()})
                $attempts | ConvertTo-Json | Set-Content (Join-Path $EvidenceRoot 'composition-attempts.json') -Encoding utf8NoBOM
                if($attempt -eq 3 -or $_.ToString() -notmatch 'Visibility changed|Foreign window|visibility changed'){throw}
                Start-Sleep -Milliseconds 300
            }
        }
        $attempts | ConvertTo-Json | Set-Content (Join-Path $EvidenceRoot 'composition-attempts.json') -Encoding utf8NoBOM
        $proof | ConvertTo-Json | Set-Content (Join-Path $EvidenceRoot 'composition.json') -Encoding utf8NoBOM
    }
    Check 'tab-and-reverse-switch-whole-space' {
        Ui 'key' @('9') | Out-Null
        Wait-Text 'ControlType.Group | Favorites space' | Out-Null
        Shot '02-favorites'
        Ui 'key' @('9','','shift') | Out-Null
        Wait-Text 'ControlType.Group | History space' | Out-Null
    }
    Check 'scoped-search-does-not-leak-between-spaces' {
        Set-Ui 'Search clipboard history' 'echo-fixture-0015'
        Wait-Text 'echo-fixture-0015@example.net' | Out-Null
        Ui 'key' @('9') | Out-Null
        Wait-Text 'No matches in this space' | Out-Null
        if((Ui 'read' @('Search clipboard history')) -ne 'echo-fixture-0015'){throw 'Switch did not preserve query.'}
        Set-Ui 'Search clipboard history' ''
    }
    Check 'create-custom-space' {
        Invoke-Ui 'New space'
        Wait-Text 'Space name' | Out-Null
        Set-Ui 'Space name' 'Scenario Work'
        Set-Ui 'Space description' 'Synthetic acceptance workspace'
        Invoke-Ui 'Save space'
        Wait-Text 'ControlType.Group | Scenario Work space' | Out-Null
        Wait-Text 'A space for things you use often' | Out-Null
        Shot '03-custom-empty'
    }
    Check 'create-content-directly-in-custom-space' {
        Invoke-Ui 'New content'
        Wait-Text 'Favorite content' | Out-Null
        Set-Ui 'Favorite name' 'Scenario Contact'
        Set-Ui 'Favorite content' 'flow-original@example.test'
        Set-Ui 'Favorite tags' 'synthetic, acceptance'
        Invoke-Ui 'Save favorite'
        Wait-Text 'flow-original@example.test' | Out-Null
        Shot '04-custom-content'
    }
    Check 'share-saved-content-with-favorites' {
        Invoke-Ui 'Item options'
        Invoke-Ui 'Add to another space…'
        Invoke-Ui 'Favorites'
        Wait-Text 'flow-original@example.test' | Out-Null
    }
    Check 'shared-editor-updates-original-content' {
        Invoke-Ui 'Edit favorite'
        Wait-Text 'Used in 2 spaces' | Out-Null
        Set-Ui 'Favorite content' 'flow-updated@example.test'
        Invoke-Ui 'Save favorite'
        Wait-Text 'flow-updated@example.test' | Out-Null
    }
    Check 'independent-copy-does-not-overwrite-shared-item' {
        Invoke-Ui 'Item options'
        Invoke-Ui 'Create an independent copy'
        Wait-Text 'Favorite content' | Out-Null
        Set-Ui 'Favorite name' 'Scenario Independent'
        Set-Ui 'Favorite content' 'flow-independent@example.test'
        Invoke-Ui 'Save favorite'
        Wait-Text 'flow-independent@example.test' | Out-Null
    }
    Check 'remove-membership-keeps-content-discoverable' {
        Set-Ui 'Search clipboard history' 'flow-updated'
        Wait-Text 'flow-updated@example.test' | Out-Null
        Invoke-Ui 'Item options'
        Invoke-Ui 'Remove from this space'
        Wait-Text 'No matches in this space' | Out-Null
        Set-Ui 'Search clipboard history' ''
        Open-Space 'Favorites'
        Set-Ui 'Search clipboard history' 'flow-updated'
        Wait-Text 'flow-updated@example.test' | Out-Null
        Set-Ui 'Search clipboard history' ''
    }
    Check 'deleting-custom-space-rehomes-exclusive-content' {
        Open-Space 'Scenario Work'
        Invoke-Ui 'Space options'
        Invoke-Ui 'Delete this space'
        Wait-Text 'No saved content is deleted' | Out-Null
        Invoke-Ui 'Delete space'
        Wait-Text 'ControlType.Group | Favorites space' | Out-Null
        Set-Ui 'Search clipboard history' 'flow-independent'
        Wait-Text 'flow-independent@example.test' | Out-Null
        Set-Ui 'Search clipboard history' ''
    }
    Check 'settings-pages-and-native-theme-preview' {
        Invoke-Ui 'Settings'
        Wait-Text 'Appearance & motion' | Out-Null
        Ui 'theme' @('light') | Out-Null
        Wait-Text 'Unsaved changes' | Out-Null
        Shot '05-settings-light'
        Invoke-Ui 'Save changes'
        Wait-Text 'Your settings are saved' | Out-Null
        foreach($label in @('Spaces & content','Keyboard & insertion','Capture & privacy','Storage & diagnostics')) {Invoke-Ui $label;Start-Sleep -Milliseconds 60}
        Shot '06-settings-diagnostics'
    }
    Check 'invalid-settings-cannot-be-saved-and-cancel-restores' {
        Set-Ui 'Maximum history entries' '0'
        Wait-Text 'greater than zero' | Out-Null
        $dump=[string](Ui 'dump')
        if($dump -notmatch 'Save changes \| enabled=False'){throw 'Invalid settings left Save enabled.'}
        Invoke-Ui 'Cancel'
        Wait-Text 'Discard unsaved changes?' | Out-Null
        Invoke-Ui 'Discard changes'
        Wait-Text 'ControlType.Group | Favorites space' | Out-Null
        Invoke-Ui 'Settings'
        Invoke-Ui 'Storage & diagnostics'
        if((Ui 'read' @('Maximum history entries')) -ne '5000'){throw 'Cancel failed to restore the persisted settings.'}
    }
    Check 'redacted-diagnostic-export-and-cache-cleanup' {
        Ensure-Control 'Export diagnostics'
        Invoke-Ui 'Export diagnostics'
        Wait-Text 'Diagnostics saved:' | Out-Null
        $files=@(Get-ChildItem (Join-Path $EvidenceRoot 'data/diagnostics') -Filter '*.json')
        if($files.Count -ne 1){throw 'Diagnostic export was not created.'}
        $json=Get-Content $files[0].FullName -Raw
        if($json -match 'flow-original|flow-updated|Scenario Contact|clipboard.*body'){throw 'Diagnostics included synthetic payload data.'}
        Invoke-Ui 'Clear motion cache'
        Wait-Text 'Motion cache cleared' | Out-Null
        Invoke-Ui 'Cancel'
        Wait-Text 'ControlType.Group | Favorites space' | Out-Null
        Shot '07-favorites-light'
    }
    Check 'closing-hides-single-window' {
        Ui 'close' | Out-Null
        Start-Sleep -Milliseconds 200
        if(Ui 'exists'){throw 'Close destroyed neither visibility nor focus as expected.'}
        & $Executable --history
        if($LASTEXITCODE -ne 0){throw 'Secondary activation failed.'}
        Wait-Text 'ControlType.Group | History space' | Out-Null
        if((Ui 'window-count') -ne 1){throw 'Reactivation created an extra window.'}
    }
    if(!$CoreOnly){. (Join-Path $Root 'tests/native/CoverFlowScenarios.ps1')}
    $success=$true
}
catch {
    $checks.Add([ordered]@{name='run';status='FAIL';error=$_.ToString()})
    Write-Output $_.ToString()
}
finally {
    if($null -ne $owned -and !$owned.HasExited) {
        & $Executable --quit
        if(!$owned.WaitForExit(10000)) {$checks.Add([ordered]@{name='graceful-exit';status='FAIL';error='Owned test instance did not exit.'});$success=$false}
        elseif($owned.ExitCode -ne 0){$checks.Add([ordered]@{name='graceful-exit';status='FAIL';error="Application exited with $($owned.ExitCode)"});$success=$false}
        else {$checks.Add([ordered]@{name='graceful-exit';status='PASS'})}
    }
    if((Get-Content (Join-Path $EvidenceRoot 'application-stderr.log') -Raw) -match 'panicked|Validation Error'){$checks.Add([ordered]@{name='runtime-errors';status='FAIL';error='Application stderr contains a panic or GPU validation error.'});$success=$false}
    $checks.Add([ordered]@{name='physical-ime';status='NOT_RUN';reason='Synthetic UI Automation cannot certify physical IME input.'})
    $checks.Add([ordered]@{name='physical-target-insertion';status='NOT_RUN';reason='This suite deliberately does not mutate the OS clipboard or paste into other applications.'})
    $checks | ConvertTo-Json -Depth 12 | Set-Content (Join-Path $EvidenceRoot 'checks.json') -Encoding utf8NoBOM
    $calls | ConvertTo-Json -Depth 12 | Set-Content (Join-Path $EvidenceRoot 'calls.json') -Encoding utf8NoBOM
    [ordered]@{schema='echo.cover-flow.acceptance.v1';status=$(if($success){'PASS'}else{'FAIL'});renderer=$Renderer;executable=$Executable;sha256=(Get-FileHash $Executable -Algorithm SHA256).Hash;synthetic=$true;finished_utc=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json | Set-Content (Join-Path $EvidenceRoot 'summary.json') -Encoding utf8NoBOM
    foreach($key in $old.Keys){[Environment]::SetEnvironmentVariable($key,$old[$key],'Process')}
}
if(!$success){exit 1}
