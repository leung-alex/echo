# Requires the owning isolated integration runner; all Chrome data is under EvidenceRoot.
$chrome=Join-Path $env:ProgramFiles 'Google/Chrome/Application/chrome.exe'
if(!(Test-Path $chrome)){Not-Run 'chrome-uia-integration' 'Chrome is not installed at the tested location.';return}
function Browser-Value {
    return F 'read-edit' @([string]$browserProcess.Id,$browserTitle,'Echo browser input')
}
function Focus-Browser {
    if(D exists){D close|Out-Null;Wait-Hidden}
    $operation=if((F geometry @([string]$browserProcess.Id,$browserTitle)).foreground){'focus-edit'}else{'focus-edit-permitted'}
    F $operation @([string]$browserProcess.Id,$browserTitle,'Echo browser input')|Out-Null
}
Check 'owned-chrome-fixture' {
    if(D exists){D close|Out-Null;Wait-Hidden}
    $info=[Diagnostics.ProcessStartInfo]::new($chrome);$info.UseShellExecute=$false
    $profile=Join-Path $EvidenceRoot 'chrome-profile'
    $html=[Uri]::new((Join-Path $Root 'tests/native/fixtures/focus-browser.html')).AbsoluteUri
    foreach($arg in @("--user-data-dir=$profile",'--no-first-run','--no-default-browser-check','--disable-background-mode','--disable-background-networking','--disable-sync','--disable-extensions',"--app=$html")){$info.ArgumentList.Add($arg)}
    $script:browserProcess=[Diagnostics.Process]::Start($info)
    Wait-Until {$browserProcess.Refresh();if($browserProcess.HasExited){throw 'Owned Chrome exited'};$browserProcess.MainWindowTitle.Contains('Echo Focus Browser Fixture')} 'owned Chrome fixture window' 15000|Out-Null
    $script:browserTitle=$browserProcess.MainWindowTitle
    Record-Owned $browserProcess $browserTitle
    Wait-Until {Focus-Browser;(Browser-Value) -eq 'ac'} 'Chrome accessibility tree and synthetic input ready' 15000|Out-Null
    'Separate Chrome profile, local fixture file, and real UIA editable input'
}
Check 'chrome-caret-anchored-hotkey-and-exact-insertion' {
    Focus-Browser
    $script:lastTargetGeometry=F geometry @([string]$browserProcess.Id,$browserTitle)
    $ruler=F card @([string]$browserProcess.Id,$browserTitle,'Expected browser caret')
    $script:lastTargetGeometry.caret=$ruler
    $latency=F 'hotkey-ready' @([string]$browserProcess.Id,$browserTitle,'Ctrl+Alt+J',[string]$echoProcess.Id,$mainTitle)
    Assert-Anchored '07-chrome-caret-popup'
    $latency|ConvertTo-Json|Set-Content (Join-Path $EvidenceRoot 'chrome-hotkey-latency.json') -Encoding utf8NoBOM
    if((D dump).Contains('Copy only')){throw 'A supported browser input was not captured as an insertion target.'}
    Query 'echo-perf-text-0013';Wait-Text 'echo-perf-text-0013'
    $payload='echo-perf-text-0013 '+[char]0x2014+' Reusable content, available when you need it.'
    Select-Row $payload;Click 'Insert item';Wait-Hidden
    Wait-Until {(Browser-Value) -eq ('a'+$payload+'c')} 'exact paste into browser caret' 10000|Out-Null
    'Ctrl+Alt+J anchored to the independently measured browser caret and pasted exact original text'
}
Check 'chrome-held-modifiers-refuse-injection-without-replay' {
    Focus-Browser
    $before=Browser-Value
    foreach($file in @('held.ready','release-held')){if(Test-Path (Join-Path $EvidenceRoot $file)){Remove-Item (Join-Path $EvidenceRoot $file)}}
    $script:held=Start-FocusProcess 'hold-hotkey' @([string]$browserProcess.Id,$browserTitle,'Ctrl+Alt+J')
    Wait-Until {Test-Path (Join-Path $EvidenceRoot 'held.ready')} 'browser held modifiers' 5000|Out-Null
    Wait-Until {D exists} 'browser hotkey popup'|Out-Null
    Query 'echo-perf-text-0013';Wait-Text 'echo-perf-text-0013'
    $payload='echo-perf-text-0013 '+[char]0x2014+' Reusable content, available when you need it.'
    Select-Row $payload;Click 'Insert item'
    Wait-Until {(D dump) -match 'modifier|release|Release|held|busy|not accept'} 'modifier-busy UIA delivery refusal' 8000|Out-Null
    if((Browser-Value) -ne $before){throw 'Browser received a paste while modifiers were held.'}
    [IO.File]::WriteAllText((Join-Path $EvidenceRoot 'release-held'),'release owned keys')
    if(!$held.WaitForExit(5000) -or $held.ExitCode -ne 0){throw 'Modifier fixture did not finish.'}
    Start-Sleep -Milliseconds 100
    if((Browser-Value) -ne $before){throw 'Failed browser insertion replayed after key release.'}
    'Real synthetic Ctrl+V delivery refused held modifiers and did not replay the operation'
}
Check 'chrome-profile-cleanup' {
    if(!$browserProcess.CloseMainWindow()){throw 'Could not close the owned browser window.'}
    if(!$browserProcess.WaitForExit(10000)){throw 'Owned Chrome did not exit.'}
    'Owned Chrome process exited; daily Chrome profile was not opened by the harness'
}
